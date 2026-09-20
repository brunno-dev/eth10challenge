//! Host side of the CUDA port: context/module management and the per-primitive
//! selftest harness that verifies each device function against the CPU crates.

use anyhow::{Context as _, Result};
use cust::context::Context;
use cust::event::{Event, EventFlags};
use cust::launch;
use cust::memory::{CopyDestination, DeviceBuffer};
use cust::module::Module;
use cust::stream::{Stream, StreamFlags};

/// PTX emitted by `build.rs` (nvcc compiling `src/cuda/kernels.cu`).
const PTX: &str = include_str!(env!("KERNELS_PTX"));

/// Owns the CUDA context + loaded module for the lifetime of a run.
pub struct Gpu {
    module: Module,
    stream: Stream,
    _context: Context,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        let _context = cust::quick_init().context("CUDA init failed (no device / driver?)")?;
        let module = Module::from_ptx(PTX, &[]).context("loading kernels PTX")?;
        let stream =
            Stream::new(StreamFlags::NON_BLOCKING, None).context("creating CUDA stream")?;

        // Build the fixed-base window table of multiples of G. Every kernel that
        // derives a public key reads it, so it must run before anything else.
        let init = module
            .get_function("k_init_gtable")
            .context("loading k_init_gtable")?;
        unsafe {
            launch!(init<<<(64u32 * 15).div_ceil(256), 256, 0, stream>>>())
                .context("launching k_init_gtable")?;
        }
        stream.synchronize().context("building G table")?;

        Ok(Self {
            _context,
            module,
            stream,
        })
    }

    /// Runs a one-message-per-thread hash kernel over `inputs`, returning a
    /// `digest_len`-byte digest per input. The kernel signature must be
    /// `(const u8* msgs, const u32* lens, u32 stride, u8* out, u32 n)`.
    fn hash_batch(
        &self,
        kernel: &str,
        inputs: &[Vec<u8>],
        digest_len: usize,
    ) -> Result<Vec<Vec<u8>>> {
        let n = inputs.len();
        let (packed, lens, stride) = pack(inputs);

        let d_msgs = DeviceBuffer::from_slice(&packed)?;
        let d_lens = DeviceBuffer::from_slice(&lens)?;
        let d_out = DeviceBuffer::from_slice(&vec![0u8; n * digest_len])?;

        let func = self.module.get_function(kernel)?;
        let (grid, block) = launch_dims(n);
        let stream = &self.stream;
        unsafe {
            launch!(func<<<grid, block, 0, stream>>>(
                d_msgs.as_device_ptr(),
                d_lens.as_device_ptr(),
                stride as u32,
                d_out.as_device_ptr(),
                n as u32
            ))?;
        }
        stream.synchronize()?;

        let mut out = vec![0u8; n * digest_len];
        d_out.copy_to(&mut out)?;
        Ok(out.chunks(digest_len).map(|c| c.to_vec()).collect())
    }

    /// One HMAC-SHA512 (64-byte output) per (key, msg) pair.
    fn hmac_batch(&self, keys: &[Vec<u8>], msgs: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
        let n = keys.len();
        let (pk, klens, kstride) = pack(keys);
        let (pm, mlens, mstride) = pack(msgs);

        let d_keys = DeviceBuffer::from_slice(&pk)?;
        let d_klens = DeviceBuffer::from_slice(&klens)?;
        let d_msgs = DeviceBuffer::from_slice(&pm)?;
        let d_mlens = DeviceBuffer::from_slice(&mlens)?;
        let d_out = DeviceBuffer::from_slice(&vec![0u8; n * 64])?;

        let func = self.module.get_function("k_hmac_sha512")?;
        let (grid, block) = launch_dims(n);
        let stream = &self.stream;
        unsafe {
            launch!(func<<<grid, block, 0, stream>>>(
                d_keys.as_device_ptr(), d_klens.as_device_ptr(), kstride as u32,
                d_msgs.as_device_ptr(), d_mlens.as_device_ptr(), mstride as u32,
                d_out.as_device_ptr(), n as u32
            ))?;
        }
        stream.synchronize()?;
        let mut out = vec![0u8; n * 64];
        d_out.copy_to(&mut out)?;
        Ok(out.chunks(64).map(|c| c.to_vec()).collect())
    }

    /// One compressed (33-byte) public key per 32-byte big-endian private key.
    fn pubkey_batch(&self, privs: &[[u8; 32]]) -> Result<Vec<[u8; 33]>> {
        let n = privs.len();
        let flat: Vec<u8> = privs.iter().flatten().copied().collect();
        let d_priv = DeviceBuffer::from_slice(&flat)?;
        let d_out = DeviceBuffer::from_slice(&vec![0u8; n * 33])?;

        let func = self.module.get_function("k_pubkey")?;
        let (grid, block) = launch_dims(n);
        let stream = &self.stream;
        unsafe {
            launch!(func<<<grid, block, 0, stream>>>(
                d_priv.as_device_ptr(), d_out.as_device_ptr(), n as u32
            ))?;
        }
        stream.synchronize()?;
        let mut out = vec![0u8; n * 33];
        d_out.copy_to(&mut out)?;
        Ok(out.chunks(33).map(|c| c.try_into().unwrap()).collect())
    }

    /// One 64-byte uncompressed public key (X||Y) per 32-byte private key.
    fn pubkey_xy_batch(&self, privs: &[[u8; 32]]) -> Result<Vec<[u8; 64]>> {
        let n = privs.len();
        let flat: Vec<u8> = privs.iter().flatten().copied().collect();
        let d_priv = DeviceBuffer::from_slice(&flat)?;
        let d_out = DeviceBuffer::from_slice(&vec![0u8; n * 64])?;

        let func = self.module.get_function("k_pubkey_xy")?;
        let (grid, block) = launch_dims(n);
        let stream = &self.stream;
        unsafe {
            launch!(func<<<grid, block, 0, stream>>>(
                d_priv.as_device_ptr(), d_out.as_device_ptr(), n as u32
            ))?;
        }
        stream.synchronize()?;
        let mut out = vec![0u8; n * 64];
        d_out.copy_to(&mut out)?;
        Ok(out.chunks(64).map(|c| c.try_into().unwrap()).collect())
    }

    /// One 20-byte Ethereum address (path m/44'/60'/0'/0/0) per 64-byte seed.
    fn seed_to_eth_batch(&self, seeds: &[[u8; 64]]) -> Result<Vec<[u8; 20]>> {
        let n = seeds.len();
        let flat: Vec<u8> = seeds.iter().flatten().copied().collect();
        let d_seed = DeviceBuffer::from_slice(&flat)?;
        let d_out = DeviceBuffer::from_slice(&vec![0u8; n * 20])?;

        let func = self.module.get_function("k_seed_to_eth")?;
        let (grid, block) = launch_dims(n);
        let stream = &self.stream;
        unsafe {
            launch!(func<<<grid, block, 0, stream>>>(
                d_seed.as_device_ptr(), d_out.as_device_ptr(), n as u32
            ))?;
        }
        stream.synchronize()?;
        let mut out = vec![0u8; n * 20];
        d_out.copy_to(&mut out)?;
        Ok(out.chunks(20).map(|c| c.try_into().unwrap()).collect())
    }

    /// One (a + b) mod n per pair, all 32-byte big-endian.
    fn binary32_batch(
        &self,
        kernel: &str,
        a: &[[u8; 32]],
        b: &[[u8; 32]],
    ) -> Result<Vec<[u8; 32]>> {
        let n = a.len();
        let fa: Vec<u8> = a.iter().flatten().copied().collect();
        let fb: Vec<u8> = b.iter().flatten().copied().collect();
        let d_a = DeviceBuffer::from_slice(&fa)?;
        let d_b = DeviceBuffer::from_slice(&fb)?;
        let d_out = DeviceBuffer::from_slice(&vec![0u8; n * 32])?;

        let func = self.module.get_function(kernel)?;
        let (grid, block) = launch_dims(n);
        let stream = &self.stream;
        unsafe {
            launch!(func<<<grid, block, 0, stream>>>(
                d_a.as_device_ptr(), d_b.as_device_ptr(), d_out.as_device_ptr(), n as u32
            ))?;
        }
        stream.synchronize()?;
        let mut out = vec![0u8; n * 32];
        d_out.copy_to(&mut out)?;
        Ok(out.chunks(32).map(|c| c.try_into().unwrap()).collect())
    }

    /// One PBKDF2-HMAC-SHA512 (dkLen=64) per (password, salt) pair.
    fn pbkdf2_batch(&self, pws: &[Vec<u8>], salts: &[Vec<u8>], iters: u32) -> Result<Vec<Vec<u8>>> {
        let n = pws.len();
        let (pp, pwlens, pwstride) = pack(pws);
        let (ps, slens, sstride) = pack(salts);

        let d_pw = DeviceBuffer::from_slice(&pp)?;
        let d_pwlens = DeviceBuffer::from_slice(&pwlens)?;
        let d_salt = DeviceBuffer::from_slice(&ps)?;
        let d_slens = DeviceBuffer::from_slice(&slens)?;
        let d_out = DeviceBuffer::from_slice(&vec![0u8; n * 64])?;

        let func = self.module.get_function("k_pbkdf2")?;
        let (grid, block) = launch_dims(n);
        let stream = &self.stream;
        unsafe {
            launch!(func<<<grid, block, 0, stream>>>(
                d_pw.as_device_ptr(), d_pwlens.as_device_ptr(), pwstride as u32,
                d_salt.as_device_ptr(), d_slens.as_device_ptr(), sstride as u32,
                iters, d_out.as_device_ptr(), n as u32
            ))?;
        }
        stream.synchronize()?;
        let mut out = vec![0u8; n * 64];
        d_out.copy_to(&mut out)?;
        Ok(out.chunks(64).map(|c| c.to_vec()).collect())
    }
}

/// Packs variable-length byte vectors into a fixed-stride buffer plus lengths.
/// Returns (packed, lens, stride). Stride is at least 1 so device pointers stay valid.
fn pack(inputs: &[Vec<u8>]) -> (Vec<u8>, Vec<u32>, usize) {
    let n = inputs.len();
    let stride = inputs.iter().map(|m| m.len()).max().unwrap_or(0).max(1);
    let mut packed = vec![0u8; n * stride];
    let mut lens = vec![0u32; n];
    for (i, m) in inputs.iter().enumerate() {
        packed[i * stride..i * stride + m.len()].copy_from_slice(m);
        lens[i] = m.len() as u32;
    }
    (packed, lens, stride)
}

fn launch_dims(n: usize) -> (u32, u32) {
    let block = 256u32;
    let grid = (n as u32).div_ceil(block);
    (grid.max(1), block)
}

/// A wordlist prepared for the GPU: NFKD bytes packed at a fixed stride plus a
/// per-word byte length. BIP-39 wordlists are already NFKD-normalized, so the
/// canonical word strings can be used verbatim.
pub struct GpuWordlist {
    packed: Vec<u8>,
    lens: Vec<u8>,
    stride: usize,
}

impl GpuWordlist {
    pub fn new(words: &[&str]) -> Result<Self> {
        let stride = words.iter().map(|w| w.len()).max().unwrap_or(1).max(1);
        // Kernel's mnemonic buffer is 512 bytes: 12 words + 11 spaces must fit.
        anyhow::ensure!(
            stride * 12 + 11 <= 512,
            "wordlist word too long for GPU mnemonic buffer (stride {stride})"
        );
        anyhow::ensure!(stride < 256, "word length exceeds u8 length field");
        let mut packed = vec![0u8; words.len() * stride];
        let mut lens = vec![0u8; words.len()];
        for (i, w) in words.iter().enumerate() {
            let b = w.as_bytes();
            packed[i * stride..i * stride + b.len()].copy_from_slice(b);
            lens[i] = b.len() as u8;
        }
        Ok(Self {
            packed,
            lens,
            stride,
        })
    }
}

/// Result of a successful GPU search: the global candidate index and its 12
/// word indices.
pub struct SearchHit {
    pub global_index: usize,
    pub indices: [u16; 12],
}

struct HostBatch {
    raw: usize,
    pruned: usize,
    exhausted: bool,
}

/// Build one bounded host batch. The expiration predicate is injected so time
/// flushes can be tested deterministically without CUDA or sleeping.
#[allow(clippy::too_many_arguments)]
fn fill_host_batch<I>(
    iterator: &mut I,
    buf: &mut Vec<u16>,
    positions: &mut Vec<usize>,
    batch_size: usize,
    left: usize,
    exclusion: Option<&crate::history::Exclusions>,
    track_positions: bool,
    pack_history: bool,
    source_step: &mut impl FnMut(&mut I, usize) -> Option<crate::candidates::Step>,
    expired: &mut impl FnMut() -> bool,
) -> HostBatch {
    buf.clear();
    positions.clear();
    let packing = pack_history && track_positions;
    let raw_cap = if packing {
        batch_size.saturating_mul(64)
    } else {
        batch_size
    }
    .min(left);
    let mut batch = HostBatch {
        raw: 0,
        pruned: 0,
        exhausted: false,
    };
    let mut last_time_check = 0;
    while batch.raw < raw_cap && buf.len() / 12 < batch_size {
        match source_step(iterator, raw_cap - batch.raw) {
            Some(crate::candidates::Step::Candidate(candidate)) => {
                if !exclusion.is_some_and(|rule| rule.contains(&candidate)) {
                    buf.extend_from_slice(&candidate);
                    if track_positions {
                        positions.push(batch.raw);
                    }
                }
                batch.raw += 1;
            }
            Some(crate::candidates::Step::Skipped(n)) => {
                assert!(n > 0 && n <= raw_cap - batch.raw);
                assert!(
                    track_positions,
                    "skipping requires original position mapping"
                );
                batch.raw += n;
                batch.pruned += n;
            }
            None => {
                batch.exhausted = true;
                break;
            }
        }
        // Count raw progress, so even one large subtree skip checks the clock.
        // This is a soft deadline: an individual source/filter call can exceed it.
        if packing && batch.raw - last_time_check >= 1024 {
            if expired() {
                break;
            }
            last_time_check = batch.raw;
        }
    }
    batch
}

impl Gpu {
    /// Searches `candidates` (an iterator of 12 word-index arrays) for one whose
    /// derived Ethereum address equals `target`. Streams in batches so memory
    /// stays flat.
    #[allow(clippy::too_many_arguments)]
    pub fn search<I: Iterator<Item = [u16; 12]> + Clone + Send + 'static>(
        &self,
        candidates: I,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        mut completed: impl FnMut(usize, &I) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        self.search_filtered(
            candidates,
            wordlist,
            target,
            batch_size,
            block,
            max_candidates,
            checksum,
            None,
            |n, _, cursor| completed(n, cursor),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search_filtered<I: Iterator<Item = [u16; 12]> + Clone + Send + 'static>(
        &self,
        candidates: I,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        exclusion: Option<crate::history::Exclusions>,
        completed: impl FnMut(usize, usize, &I) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        self.search_filtered_mode(
            candidates,
            wordlist,
            target,
            batch_size,
            block,
            max_candidates,
            checksum,
            exclusion,
            true,
            completed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search_filtered_mode<I: Iterator<Item = [u16; 12]> + Clone + Send + 'static>(
        &self,
        candidates: I,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        exclusion: Option<crate::history::Exclusions>,
        pack_history: bool,
        mut completed: impl FnMut(usize, usize, &I) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        let track_positions = exclusion.is_some();
        self.search_steps(
            candidates,
            wordlist,
            target,
            batch_size,
            block,
            max_candidates,
            checksum,
            exclusion,
            track_positions,
            pack_history,
            |it, _| it.next().map(crate::candidates::Step::Candidate),
            |raw, excluded, _, cursor| completed(raw, excluded, cursor),
        )
    }

    // Preserve the default-packing entry point alongside the CLI's explicit mode.
    #[allow(dead_code, clippy::too_many_arguments)]
    pub fn search_pruned(
        &self,
        candidates: crate::candidates::Candidates,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        exclusion: Option<crate::history::Exclusions>,
        pruning: std::sync::Arc<crate::pruning::Pruning>,
        completed: impl FnMut(usize, usize, usize, &crate::candidates::Candidates) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        self.search_pruned_mode(
            candidates,
            wordlist,
            target,
            batch_size,
            block,
            max_candidates,
            checksum,
            exclusion,
            pruning,
            true,
            completed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search_pruned_mode(
        &self,
        candidates: crate::candidates::Candidates,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        exclusion: Option<crate::history::Exclusions>,
        pruning: std::sync::Arc<crate::pruning::Pruning>,
        pack_history: bool,
        completed: impl FnMut(usize, usize, usize, &crate::candidates::Candidates) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        let track_positions = exclusion.is_some() || !pruning.is_empty();
        self.search_steps(
            candidates,
            wordlist,
            target,
            batch_size,
            block,
            max_candidates,
            checksum,
            exclusion,
            track_positions,
            pack_history,
            move |it, max_raw| it.next_pruned(max_raw, pruning.as_ref()),
            completed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn search_steps<I: Iterator<Item = [u16; 12]> + Clone + Send + 'static>(
        &self,
        candidates: I,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        exclusion: Option<crate::history::Exclusions>,
        track_positions: bool,
        pack_history: bool,
        source_step: impl FnMut(&mut I, usize) -> Option<crate::candidates::Step> + Send + 'static,
        completed: impl FnMut(usize, usize, usize, &I) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        self.search_steps_observed(
            candidates,
            wordlist,
            target,
            batch_size,
            block,
            max_candidates,
            checksum,
            exclusion,
            track_positions,
            pack_history,
            false,
            None,
            &mut crate::metrics::SearchMetrics::default(),
            source_step,
            completed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search_observed(
        &self,
        candidates: crate::candidates::Candidates,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        exclusion: Option<crate::history::Exclusions>,
        pruning: Option<std::sync::Arc<crate::pruning::Pruning>>,
        pack_history: bool,
        adaptive: bool,
        stop_file: Option<std::path::PathBuf>,
        metrics: &mut crate::metrics::SearchMetrics,
        completed: impl FnMut(usize, usize, usize, &crate::candidates::Candidates) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        let track_positions =
            exclusion.is_some() || pruning.as_ref().is_some_and(|p| !p.is_empty());
        self.search_steps_observed(
            candidates,
            wordlist,
            target,
            batch_size,
            block,
            max_candidates,
            checksum,
            exclusion,
            track_positions,
            pack_history,
            adaptive,
            stop_file,
            metrics,
            move |it, max_raw| match &pruning {
                Some(pruning) => it.next_pruned(max_raw, pruning.as_ref()),
                None => it.next().map(crate::candidates::Step::Candidate),
            },
            completed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn search_steps_observed<I: Iterator<Item = [u16; 12]> + Clone + Send + 'static>(
        &self,
        candidates: I,
        wordlist: &GpuWordlist,
        target: &[u8; 20],
        batch_size: usize,
        block: u32,
        max_candidates: usize,
        checksum: bool,
        exclusion: Option<crate::history::Exclusions>,
        track_positions: bool,
        pack_history: bool,
        adaptive: bool,
        stop_file: Option<std::path::PathBuf>,
        metrics: &mut crate::metrics::SearchMetrics,
        mut source_step: impl FnMut(&mut I, usize) -> Option<crate::candidates::Step> + Send + 'static,
        mut completed: impl FnMut(usize, usize, usize, &I) -> Result<()>,
    ) -> Result<Option<SearchHit>> {
        anyhow::ensure!(
            batch_size > 0 && batch_size <= (u32::MAX / 12) as usize,
            "invalid batch size"
        );
        anyhow::ensure!([32, 64, 128, 256].contains(&block), "invalid block size");
        if max_candidates == 0 {
            return Ok(None);
        }
        let d_wordlist = DeviceBuffer::from_slice(&wordlist.packed)?;
        let d_lens = DeviceBuffer::from_slice(&wordlist.lens)?;
        let d_target = DeviceBuffer::from_slice(target)?;
        let filter = self.module.get_function("k_filter")?;
        let seed_kernel = self.module.get_function("k_candidate_seeds")?;
        let address_kernel = self.module.get_function("k_seed_addresses")?;
        // Events are reused, recorded on the same stream, and queried only
        // after the existing final synchronization. Do not add a host barrier
        // between seed and address kernels just to time them.
        let seed_start_event = Event::new(EventFlags::DEFAULT)?;
        let seed_end_event = Event::new(EventFlags::DEFAULT)?;
        let address_end_event = Event::new(EventFlags::DEFAULT)?;
        metrics.gpu_seed_seconds = Some(0.0);
        metrics.gpu_address_seconds = Some(0.0);

        // Reuse device buffers across batches. Only the survivor seed buffer
        // may grow; cudaMalloc/cudaFree implicitly synchronize the device.
        let d_survivors = unsafe { DeviceBuffer::<u32>::uninitialized(batch_size)? };
        let d_cand = unsafe { DeviceBuffer::<u16>::uninitialized(batch_size * 12)? };
        // Grow only if a batch has more checksum survivors than seen so far.
        // Arbitrary caller input can be 100% valid; never assume exactly 1/16.
        let mut d_seeds = unsafe { DeviceBuffer::<u8>::uninitialized(0)? };
        let mut d_counter = DeviceBuffer::from_slice(&[0u32])?;
        let d_found_flag = DeviceBuffer::from_slice(&[0u32])?;
        let d_found_idx = DeviceBuffer::from_slice(&[0u32])?;

        // Generating a batch takes ~10-15% of the time the GPU spends on it, and
        // it used to run between launches with the device idle. A producer thread
        // fills the next batch while the current one is in flight; buffers cycle
        // back over `empty` so nothing is reallocated.
        let mut controller = crate::adaptive::BatchController::new(batch_size);
        let desired = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(if adaptive {
            controller.size()
        } else {
            batch_size
        }));
        let producer_desired = desired.clone();
        let (full_tx, full_rx) = std::sync::mpsc::sync_channel::<(
            Vec<u16>,
            Vec<usize>,
            I,
            HostBatch,
            bool,
            usize,
            f64,
        )>(1);
        let (empty_tx, empty_rx) = std::sync::mpsc::channel::<(Vec<u16>, Vec<usize>)>();
        for _ in 0..2 {
            let _ = empty_tx.send((Vec::with_capacity(batch_size * 12), Vec::new()));
        }
        let producer = std::thread::spawn(move || {
            let mut it = candidates;
            let mut left = max_candidates;
            while let Ok((mut buf, mut positions)) = empty_rx.recv() {
                let started = std::time::Instant::now();
                let requested = producer_desired.load(std::sync::atomic::Ordering::Relaxed);
                let batch = fill_host_batch(
                    &mut it,
                    &mut buf,
                    &mut positions,
                    requested,
                    left,
                    exclusion.as_ref(),
                    track_positions,
                    pack_history,
                    &mut source_step,
                    &mut || started.elapsed() >= std::time::Duration::from_millis(250),
                );
                left -= batch.raw;
                let last = batch.exhausted || left == 0;
                let producer_seconds = started.elapsed().as_secs_f64();
                // A send error just means the consumer stopped (hit found).
                if full_tx
                    .send((
                        buf,
                        positions,
                        it.clone(),
                        batch,
                        last,
                        requested,
                        producer_seconds,
                    ))
                    .is_err()
                    || last
                {
                    return;
                }
            }
        });

        let mut batch_start: usize = 0;
        let mut checked: usize = 0;
        let stream = &self.stream;

        let mut last_print = std::time::Instant::now();
        let result = (|| -> Result<Option<SearchHit>> {
            let result = loop {
                if stop_file.as_ref().is_some_and(|path| path.exists()) {
                    break None;
                }
                let waiting = std::time::Instant::now();
                let (buf, positions, cursor, batch, last, requested, producer_seconds) =
                    match full_rx.recv() {
                        Ok(b) => b,
                        Err(_) => break None, // producer finished
                    };
                let wait_seconds = waiting.elapsed().as_secs_f64();
                let original_n = batch.raw;
                let pruned = batch.pruned;
                if original_n == 0 {
                    break None;
                }
                let n = buf.len() / 12;
                if n == 0 {
                    let checkpoint_start = std::time::Instant::now();
                    completed(original_n, original_n, pruned, &cursor)?;
                    metrics.checkpoint_seconds += checkpoint_start.elapsed().as_secs_f64();
                    metrics.record_batch(original_n, 0, pruned, 0)?;
                    metrics.observe_batch_size(requested);
                    metrics.producer_seconds += producer_seconds;
                    metrics.wait_seconds += wait_seconds;
                    checked += original_n;
                    batch_start += original_n;
                    let _ = empty_tx.send((buf, positions));
                    if last {
                        break None;
                    }
                    continue;
                }

                let device_start = std::time::Instant::now();
                let transfer_start = std::time::Instant::now();
                d_cand.index(0..buf.len()).copy_from(&buf[..])?;
                d_counter.copy_from(&[0u32])?;
                let transfer_seconds = transfer_start.elapsed().as_secs_f64();

                // Pass 1: checksum filter -> compacted survivors.
                let grid = (n as u32).div_ceil(block);
                let filter_start = std::time::Instant::now();
                unsafe {
                    launch!(filter<<<grid, block, 0, stream>>>(
                        d_cand.as_device_ptr(), n as u32,
                        d_survivors.as_device_ptr(), d_counter.as_device_ptr(), checksum as u32
                    ))?;
                }
                stream.synchronize()?;
                let mut counter = [0u32];
                d_counter.copy_to(&mut counter)?;
                let count = counter[0];
                let filter_seconds = filter_start.elapsed().as_secs_f64();

                // Pass 2: heavy derivation over survivors only.
                let derive_start = std::time::Instant::now();
                let mut seed_seconds = 0.0;
                let mut address_seconds = 0.0;
                if count > 0 {
                    let seed_bytes = count as usize * 64;
                    if d_seeds.len() < seed_bytes {
                        let capacity = seed_bytes.max(d_seeds.len().saturating_mul(2));
                        d_seeds = unsafe { DeviceBuffer::uninitialized(capacity)? };
                    }
                    let grid2 = count.div_ceil(block);
                    seed_start_event.record(stream)?;
                    unsafe {
                        launch!(seed_kernel<<<grid2, block, 0, stream>>>(
                            d_cand.as_device_ptr(),
                            d_survivors.as_device_ptr(),
                            count,
                            d_wordlist.as_device_ptr(),
                            d_lens.as_device_ptr(),
                            wordlist.stride as u32,
                            d_seeds.as_device_ptr()
                        ))?;
                    }
                    seed_end_event.record(stream)?;
                    unsafe {
                        launch!(address_kernel<<<grid2, block, 0, stream>>>(
                            d_seeds.as_device_ptr(),
                            d_survivors.as_device_ptr(),
                            count,
                            d_target.as_device_ptr(),
                            d_found_flag.as_device_ptr(),
                            d_found_idx.as_device_ptr()
                        ))?;
                    }
                    address_end_event.record(stream)?;
                    stream.synchronize()?;
                    seed_seconds = seed_end_event.elapsed_time_f32(&seed_start_event)? as f64 / 1000.0;
                    address_seconds = address_end_event.elapsed_time_f32(&seed_end_event)? as f64 / 1000.0;
                }
                let derive_seconds = if count > 0 {
                    derive_start.elapsed().as_secs_f64()
                } else {
                    0.0
                };

                let mut found_flag = [0u32];
                d_found_flag.copy_to(&mut found_flag)?;
                if found_flag[0] != 0 {
                    let mut found_idx = [0u32];
                    d_found_idx.copy_to(&mut found_idx)?;
                    let local = found_idx[0] as usize;
                    anyhow::ensure!(local < n, "GPU returned an out-of-range candidate index");
                    let mut indices = [0u16; 12];
                    indices.copy_from_slice(&buf[local * 12..local * 12 + 12]);
                    break Some(SearchHit {
                        global_index: batch_start
                            + if track_positions {
                                positions[local]
                            } else {
                                local
                            },
                        indices,
                    });
                }

                let device_elapsed = device_start.elapsed();
                let checkpoint_start = std::time::Instant::now();
                completed(original_n, original_n - n, pruned, &cursor)?;
                metrics.checkpoint_seconds += checkpoint_start.elapsed().as_secs_f64();
                metrics.record_batch(original_n, n, pruned, count as usize)?;
                metrics.device_batches += 1;
                metrics.observe_batch_size(requested);
                metrics.producer_seconds += producer_seconds;
                metrics.wait_seconds += wait_seconds;
                metrics.transfer_seconds += transfer_seconds;
                metrics.filter_seconds += filter_seconds;
                metrics.derive_seconds += derive_seconds;
                *metrics.gpu_seed_seconds.as_mut().unwrap() += seed_seconds;
                *metrics.gpu_address_seconds.as_mut().unwrap() += address_seconds;
                if adaptive {
                    let before = controller.size();
                    let next = controller.observe(requested, n, device_elapsed);
                    if next != before {
                        metrics.adaptive_changes += 1;
                        desired.store(next, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                checked += original_n;
                batch_start += original_n;
                let _ = empty_tx.send((buf, positions));
                if last_print.elapsed().as_secs() >= 2 {
                    println!("Checked {} candidates...", crate::format_number(checked));
                    last_print = std::time::Instant::now();
                }
                use std::io::Write;
                let _ = std::io::stdout().flush();
                if last {
                    break None;
                }
            };

            Ok(result)
        })();

        // Break out of the loop early (a hit) and the producer may be parked in
        // either direction, so both of its peers have to go before the join:
        // dropping the receiver fails its pending send, dropping the sender ends
        // its wait for the next empty buffer.
        drop(full_rx);
        drop(empty_tx);
        producer
            .join()
            .map_err(|_| anyhow::anyhow!("candidate producer panicked; search incomplete"))?;
        result
    }
}

/// Runs all primitive selftests, printing PASS/FAIL per primitive. Returns
/// `Ok(true)` iff every check passed.
pub fn run_selftest() -> Result<bool> {
    use bitcoin::hashes::{sha256, sha512, Hash};

    let gpu = Gpu::new()?;
    println!(
        "GPU selftest: {}",
        cust::context::CurrentContext::get_device()?.name()?
    );
    for name in ["k_candidate_seeds", "k_seed_addresses"] {
        let registers = gpu
            .module
            .get_function(name)?
            .get_attribute(cust::function::FunctionAttribute::NumRegisters)?;
        println!("  {name}: {registers} registers/thread (driver JIT)");
    }
    let mut all_ok = true;

    // Messages chosen to exercise: empty, short, multi-block boundaries.
    let mut msgs: Vec<Vec<u8>> = vec![
        vec![],
        b"abc".to_vec(),
        b"message digest".to_vec(),
        b"The quick brown fox jumps over the lazy dog".to_vec(),
        vec![0x61u8; 55], // one-block-1-byte boundary for sha256
    ];
    msgs.push(vec![0x5au8; 64]);
    msgs.push(vec![0xa5u8; 119]);
    // Keccak's rate is 136 bytes, so straddle it in both directions: 135 leaves
    // exactly one byte of padding (where the two pad marks collide into 0x81),
    // 136 forces an entire extra padding block, and 137 spills into a second.
    msgs.push(vec![0x61u8; 135]);
    msgs.push(vec![0x61u8; 136]);
    msgs.push(vec![0x61u8; 137]);
    msgs.push(vec![0u8; 200]);

    // --- SHA-256 ---
    let got = gpu.hash_batch("k_sha256", &msgs, 32)?;
    let sha256_ok = msgs.iter().zip(&got).all(|(m, g)| {
        let want = sha256::Hash::hash(m).to_byte_array();
        g.as_slice() == want
    });
    report("SHA-256", sha256_ok, &mut all_ok);

    // --- SHA-512 ---
    let got = gpu.hash_batch("k_sha512", &msgs, 64)?;
    let sha512_ok = msgs.iter().zip(&got).all(|(m, g)| {
        let want = sha512::Hash::hash(m).to_byte_array();
        g.as_slice() == want
    });
    report("SHA-512", sha512_ok, &mut all_ok);

    // --- Keccak-256 (vs the sha3 crate, plus a hardcoded known-answer test) ---
    let got = gpu.hash_batch("k_keccak256", &msgs, 32)?;
    let mut keccak_ok = msgs
        .iter()
        .zip(&got)
        .all(|(m, g)| g.as_slice() == crate::eth::keccak256(m));
    // Independent of both implementations: if the sha3 crate were somehow the
    // NIST variant, every comparison above would still agree while every digest
    // was wrong. These two constants are the published Keccak-256 vectors.
    keccak_ok &=
        hex::encode(&got[0]) == "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";
    keccak_ok &=
        hex::encode(&got[1]) == "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45";
    report(
        "Keccak-256 (incl. rate-boundary + KAT)",
        keccak_ok,
        &mut all_ok,
    );

    // --- HMAC-SHA512 (vs bitcoin_hashes HmacEngine) ---
    // Keys chosen to cross the 128-byte block boundary (short, exactly-block, oversized).
    let hkeys: Vec<Vec<u8>> = vec![
        b"key".to_vec(),
        b"Bitcoin seed".to_vec(),
        vec![0x0bu8; 20],
        vec![0xaau8; 131], // > block size -> key gets hashed first
    ];
    let hmsgs: Vec<Vec<u8>> = vec![
        b"The quick brown fox jumps over the lazy dog".to_vec(),
        vec![0x00u8; 64],
        b"Hi There".to_vec(),
        vec![0xddu8; 200],
    ];
    let got = gpu.hmac_batch(&hkeys, &hmsgs)?;
    let hmac_ok = hkeys.iter().zip(&hmsgs).zip(&got).all(|((k, m), g)| {
        use bitcoin::hashes::HashEngine;
        use bitcoin::hashes::{Hmac, HmacEngine};
        let mut eng = HmacEngine::<sha512::Hash>::new(k);
        eng.input(m);
        let want = Hmac::<sha512::Hash>::from_engine(eng).to_byte_array();
        g.as_slice() == want
    });
    report("HMAC-SHA512", hmac_ok, &mut all_ok);

    // --- PBKDF2-HMAC-SHA512 / BIP-39 seed (vs bip39 crate Mnemonic::to_seed) ---
    use bip39::{Language, Mnemonic};
    let phrases = [
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
        "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
    ];
    let mut pws = Vec::new();
    let mut salts = Vec::new();
    let mut want_seeds = Vec::new();
    for p in phrases {
        let m = Mnemonic::parse_in_normalized(Language::English, p)?;
        pws.push(m.to_string().into_bytes());
        salts.push(b"mnemonic".to_vec());
        want_seeds.push(m.to_seed("").to_vec());
    }
    // A Japanese mnemonic is ~220 bytes of UTF-8, past HMAC's 128-byte block, so
    // the key gets hashed before use. English phrases never reach that path.
    // Built from entropy rather than a literal: the wordlist is NFKD, in which
    // dakuten are separate code points, so a composed literal would not match.
    let m = Mnemonic::from_entropy_in(Language::Japanese, &[0u8; 16])?;
    let jp_bytes = m.to_string().into_bytes();
    anyhow::ensure!(
        jp_bytes.len() > 128,
        "expected the Japanese mnemonic to exceed one HMAC block, got {}",
        jp_bytes.len()
    );
    pws.push(jp_bytes);
    salts.push(b"mnemonic".to_vec());
    want_seeds.push(m.to_seed("").to_vec());

    let got = gpu.pbkdf2_batch(&pws, &salts, 2048)?;
    let pbkdf2_ok = got.iter().zip(&want_seeds).all(|(g, w)| g == w);
    report("PBKDF2-HMAC-SHA512 / BIP-39 seed", pbkdf2_ok, &mut all_ok);

    // Independent arbitrary-precision reference for the Windows carry port.
    // Boundary pairs force propagation that uniformly random keys rarely cover.
    use num_bigint::BigUint;
    let prime = BigUint::parse_bytes(
        b"fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f",
        16,
    )
    .unwrap();
    let order = BigUint::parse_bytes(
        b"fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
        16,
    )
    .unwrap();
    let encode = |n: &BigUint| {
        let bytes = n.to_bytes_be();
        let mut out = [0u8; 32];
        out[32 - bytes.len()..].copy_from_slice(&bytes);
        out
    };
    let mut edges = vec![
        BigUint::from(0u8),
        BigUint::from(1u8),
        &prime - 1u8,
        &prime - 2u8,
        &order - 1u8,
        &order - 2u8,
    ];
    for bit in [32usize, 64, 128, 192, 255] {
        let n: BigUint = BigUint::from(1u8) << bit;
        edges.extend([&n - 1u8, n.clone(), &n + 1u8]);
    }
    let mut rng_field = SplitMix64::new(12345);
    let mut pairs: Vec<_> = edges
        .iter()
        .flat_map(|a| edges.iter().map(move |b| (a.clone(), b.clone())))
        .collect();
    for _ in 0..2048 {
        pairs.push((
            BigUint::from_bytes_be(&rng_field.fill32()) % &prime,
            BigUint::from_bytes_be(&rng_field.fill32()) % &prime,
        ));
    }
    let a: Vec<_> = pairs.iter().map(|(a, _)| encode(a)).collect();
    let b: Vec<_> = pairs.iter().map(|(_, b)| encode(b)).collect();
    for kernel in ["k_fe_add", "k_fe_sub", "k_fe_mul", "k_fe_inverse_check"] {
        let got = gpu.binary32_batch(kernel, &a, &b)?;
        let ok = pairs.iter().zip(&got).all(|((a, b), got)| {
            let want = match kernel {
                "k_fe_add" => (a + b) % &prime,
                "k_fe_sub" => (a + &prime - b) % &prime,
                "k_fe_mul" => (a * b) % &prime,
                _ => a.modpow(&(&prime - 2u8), &prime), // includes defined helper behavior at zero
            };
            *got == encode(&want)
        });
        report(kernel, ok, &mut all_ok);
    }
    let scalar_a: Vec<_> = pairs.iter().map(|(a, _)| encode(&(a % &order))).collect();
    let scalar_b: Vec<_> = pairs.iter().map(|(_, b)| encode(&(b % &order))).collect();
    let got = gpu.binary32_batch("k_scalar_add", &scalar_a, &scalar_b)?;
    let ok = pairs
        .iter()
        .zip(&got)
        .all(|((a, b), g)| *g == encode(&((a + b) % &order)));
    report("scalar carry boundaries", ok, &mut all_ok);

    // --- secp256k1: priv -> compressed pubkey (vs secp256k1 crate) ---
    use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let mut rng = SplitMix64::new(0x9E3779B97F4A7C15);
    let mut privs: Vec<[u8; 32]> = Vec::new();
    // Anchor with priv == 1 (pubkey == G) for a deterministic sanity point.
    let mut one = [0u8; 32];
    one[31] = 1;
    privs.push(one);
    while privs.len() < 512 {
        let cand = rng.fill32();
        if SecretKey::from_slice(&cand).is_ok() {
            privs.push(cand);
        }
    }
    let got = gpu.pubkey_batch(&privs)?;
    let pubkey_ok = privs.iter().zip(&got).all(|(p, g)| {
        let sk = SecretKey::from_slice(p).unwrap();
        let want = PublicKey::from_secret_key(&secp, &sk).serialize();
        g == &want
    });
    report(
        "secp256k1 priv->compressed pubkey (512 keys)",
        pubkey_ok,
        &mut all_ok,
    );

    // --- secp256k1: priv -> uncompressed X||Y ---
    // The compressed form above keeps only the parity of Y, so it cannot catch a
    // Y coordinate left in a non-canonical (unreduced) representation. Ethereum
    // hashes all 32 bytes of Y, so every bit of it has to be checked.
    let got = gpu.pubkey_xy_batch(&privs)?;
    let pubkey_xy_ok = privs.iter().zip(&got).all(|(p, g)| {
        let sk = SecretKey::from_slice(p).unwrap();
        let want = PublicKey::from_secret_key(&secp, &sk).serialize_uncompressed();
        g[..] == want[1..] // drop the 0x04 prefix, which Ethereum does not hash
    });
    report(
        "secp256k1 priv->uncompressed X||Y (512 keys)",
        pubkey_xy_ok,
        &mut all_ok,
    );

    // --- scalar add mod n (vs SecretKey::add_tweak) ---
    let mut avec: Vec<[u8; 32]> = Vec::new();
    let mut bvec: Vec<[u8; 32]> = Vec::new();
    let mut want_sum: Vec<[u8; 32]> = Vec::new();
    while avec.len() < 256 {
        let a = rng.fill32();
        let b = rng.fill32();
        let (sk, tw) = match (SecretKey::from_slice(&a), Scalar::from_be_bytes(b)) {
            (Ok(s), Ok(t)) => (s, t),
            _ => continue,
        };
        let sum = match sk.add_tweak(&tw) {
            Ok(s) => s,
            Err(_) => continue, // result was zero; vanishingly rare
        };
        avec.push(a);
        bvec.push(b);
        want_sum.push(sum.secret_bytes());
    }
    let got = gpu.binary32_batch("k_scalar_add", &avec, &bvec)?;
    let addn_ok = got.iter().zip(&want_sum).all(|(g, w)| g == w);
    report(
        "secp256k1 scalar add mod n (256 pairs)",
        addn_ok,
        &mut all_ok,
    );

    // --- BIP32 m/44'/60'/0'/0/0 seed -> Ethereum address (vs the CPU path) ---
    use bitcoin::bip32::DerivationPath;
    let path: DerivationPath = crate::eth::ETH_PATH.parse()?;
    let mut seeds: Vec<[u8; 64]> = Vec::new();
    let mut want_addr: Vec<[u8; 20]> = Vec::new();
    for p in phrases {
        let m = Mnemonic::parse_in_normalized(Language::English, p)?;
        let seed = m.to_seed("");
        want_addr.push(crate::eth::address_from_seed(&secp, &path, &seed)?);
        seeds.push(seed);
    }
    let got = gpu.seed_to_eth_batch(&seeds)?;
    let bip32_ok = got.iter().zip(&want_addr).all(|(g, w)| g == w);
    report(
        "BIP32 m/44'/60'/0'/0/0 seed->ETH address",
        bip32_ok,
        &mut all_ok,
    );

    // The CPU reference above and the GPU share no code, but they do share this
    // author's reading of BIP-44. Anchor the whole chain to a value produced
    // outside both: the canonical BIP-39 test mnemonic's first MetaMask account.
    let kat_seed = Mnemonic::parse_in_normalized(Language::English, phrases[0])?.to_seed("");
    let kat = gpu.seed_to_eth_batch(&[kat_seed])?;
    let kat_ok = crate::eth::to_eip55(&kat[0]) == "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";
    report(
        "BIP32 known-answer ('abandon...about' -> MetaMask #0)",
        kat_ok,
        &mut all_ok,
    );

    // Exercise the actual filter + pipeline + host batching, including Unicode
    // phrases, partial batches, negative targets and matches across boundaries.
    let mut pipeline_ok = true;
    for &lang in Language::ALL {
        let m = Mnemonic::from_entropy_in(lang, &[0u8; 16])?;
        let indices: [u16; 12] = m
            .word_indices()
            .map(|i| i as u16)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let target = crate::eth::address_from_seed(&secp, &path, &m.to_seed_normalized(""))?;
        let words = GpuWordlist::new(lang.word_list())?;
        let mut invalid = indices;
        invalid[11] ^= 1; // flip checksum only
        for (batch, position) in [(1, 0), (32, 31), (32, 32), (64, 64)] {
            let mut inputs = vec![invalid; position + 3];
            inputs[position] = indices;
            let hit = gpu.search(
                inputs.into_iter(),
                &words,
                &target,
                batch,
                64,
                usize::MAX,
                true,
                |_, _| Ok(()),
            )?;
            pipeline_ok &= hit.is_some_and(|h| h.global_index == position && h.indices == indices);
        }
        let negative = gpu.search(
            vec![invalid, indices].into_iter(),
            &words,
            &[0u8; 20],
            3,
            64,
            usize::MAX,
            true,
            |_, _| Ok(()),
        )?;
        pipeline_ok &= negative.is_none();
        // Invalid checksum, valid word indices: pass-through must derive the
        // exact original words. Rebuilding from entropy would change the target.
        let phrase = invalid
            .iter()
            .map(|&i| lang.word_list()[i as usize])
            .collect::<Vec<_>>()
            .join(" ");
        let raw = Mnemonic::parse_in_normalized_without_checksum_check(lang, &phrase)?;
        let raw_target = crate::eth::address_from_seed(&secp, &path, &raw.to_seed_normalized(""))?;
        for checksum in [true, false] {
            let hit = gpu.search(
                vec![invalid; 3].into_iter(),
                &words,
                &raw_target,
                2,
                64,
                usize::MAX,
                checksum,
                |_, _| Ok(()),
            )?;
            pipeline_ok &= if checksum {
                hit.is_none()
            } else {
                hit.is_some_and(|h| h.indices == invalid)
            };
        }
    }
    // Dense survivors exercise compaction, concurrent PBKDF2, and different
    // block shapes; the matching phrase is surrounded by other valid phrases.
    let mut dense: Vec<[u16; 12]> = Vec::new();
    for byte in 0..129u8 {
        let m = Mnemonic::from_entropy_in(Language::English, &[byte; 16])?;
        dense.push(
            m.word_indices()
                .map(|i| i as u16)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        );
    }
    let m = Mnemonic::from_entropy_in(Language::English, &[96u8; 16])?;
    let target = crate::eth::address_from_seed(&secp, &path, &m.to_seed_normalized(""))?;
    let words = GpuWordlist::new(Language::English.word_list())?;
    let indices = dense[96];
    let mut slots = indices.map(crate::candidates::Slot::Fixed);
    slots[4..8].fill(crate::candidates::Slot::Hole);
    let two_pools = crate::candidates::stream_two_pools(
        slots,
        2,
        indices[4..6].to_vec(),
        indices[6..8].to_vec(),
    );
    let batch_hit = gpu.search(
        two_pools,
        &words,
        &target,
        3,
        64,
        usize::MAX,
        true,
        |_, _| Ok(()),
    )?;
    pipeline_ok &= batch_hit.is_some_and(|hit| hit.indices == indices);
    for block in [32, 64, 128, 256] {
        let hit = gpu.search(
            dense.clone().into_iter(),
            &words,
            &target,
            65,
            block,
            usize::MAX,
            true,
            |_, _| Ok(()),
        )?;
        pipeline_ok &= hit.is_some_and(|h| h.global_index == 96 && h.indices == dense[96]);
    }
    report(
        "full search pipeline (all languages, dense survivors, blocks and negatives)",
        pipeline_ok,
        &mut all_ok,
    );
    let mut completed_count = 0;
    let mut saved_cursor = dense.clone().into_iter();
    let limited = gpu.search(
        dense.clone().into_iter(),
        &words,
        &[0u8; 20],
        17,
        64,
        41,
        true,
        |n, cursor| {
            completed_count += n;
            saved_cursor = cursor.clone();
            Ok(())
        },
    )?;
    let limit_ok = limited.is_none()
        && completed_count == 41
        && saved_cursor.collect::<Vec<_>>() == dense[41..];
    let callback_error = gpu.search(
        dense.into_iter(),
        &words,
        &[0u8; 20],
        17,
        64,
        usize::MAX,
        true,
        |_, _| anyhow::bail!("intentional selftest checkpoint failure"),
    );
    let cleanup_ok = callback_error.is_err_and(|e| e.to_string().contains("intentional selftest"));
    report(
        "candidate limit, completed cursor and producer error cleanup",
        limit_ok && cleanup_ok,
        &mut all_ok,
    );
    // RO1 host compaction: full excluded batches, mixed batches, original hit
    // index, raw candidate limits, cursor and callback failure cleanup.
    let ro1 = crate::history::Exclusions::ro1()?;
    let excluded: [u16; 12] =
        "dutch update winter cattle fog lake also forest wood fiber fork parrot"
            .split_whitespace()
            .map(|w| Language::English.word_list().binary_search(&w).unwrap() as u16)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
    let witness = Mnemonic::from_entropy_in(Language::English, &[0x53; 16])?;
    let witness_ids: [u16; 12] = witness
        .word_indices()
        .map(|i| i as u16)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let witness_target = crate::eth::address_from_seed(
        &bitcoin::secp256k1::Secp256k1::new(),
        &crate::eth::ETH_PATH.parse()?,
        &witness.to_seed_normalized(""),
    )?;
    let mut rows = vec![excluded; 9];
    rows.push(witness_ids);
    let mut history_ok = ro1.contains(&excluded) && !ro1.contains(&witness_ids);
    for (batch, pack_history) in [1, 4, 16]
        .into_iter()
        .flat_map(|batch| [false, true].map(|packing| (batch, packing)))
    {
        let hit = gpu.search_filtered_mode(
            rows.clone().into_iter(),
            &words,
            &witness_target,
            batch,
            64,
            10,
            true,
            Some(ro1.clone()),
            pack_history,
            |_, _, _| Ok(()),
        )?;
        history_ok &= hit.is_some_and(|h| h.global_index == 9 && h.indices == witness_ids);
    }
    let mut raw = 0;
    let mut skipped = 0;
    let mut cursor = rows.clone().into_iter();
    let hit = gpu.search_filtered(
        rows.clone().into_iter(),
        &words,
        &witness_target,
        4,
        64,
        9,
        true,
        Some(ro1.clone()),
        |n, excluded, next| {
            raw += n;
            skipped += excluded;
            cursor = next.clone();
            Ok(())
        },
    )?;
    history_ok &= hit.is_none() && raw == 9 && skipped == 9 && cursor.next() == Some(witness_ids);
    let hit = gpu.search_filtered(
        rows.clone().into_iter(),
        &words,
        &[0; 20],
        16,
        64,
        10,
        true,
        Some(ro1.clone()),
        |n, excluded, _| {
            history_ok &= n == 10 && excluded == 9;
            Ok(())
        },
    )?;
    history_ok &= hit.is_none();
    let error = gpu.search_filtered(
        rows.into_iter(),
        &words,
        &[0; 20],
        4,
        64,
        10,
        true,
        Some(ro1),
        |_, _, _| anyhow::bail!("intentional history checkpoint failure"),
    );
    history_ok &= error.is_err_and(|e| e.to_string().contains("intentional history"));
    report(
        "RO1 exclusion, original indices, empty batches and resume cursor",
        history_ok,
        &mut all_ok,
    );

    // Interleaved exclusions should produce fewer nonempty device batches,
    // while completion totals and the final cursor remain identical.
    let interleaved: Vec<_> = (0..100)
        .map(|i| if i % 10 == 9 { witness_ids } else { excluded })
        .collect();
    let mut packing_ok = true;
    for packing in [false, true] {
        let mut raw = 0;
        let mut excluded_count = 0;
        let mut batches = 0;
        let mut nonempty_batches = 0;
        let mut cursor = interleaved.clone().into_iter();
        let hit = gpu.search_filtered_mode(
            interleaved.clone().into_iter(),
            &words,
            &[0; 20],
            4,
            64,
            usize::MAX,
            true,
            Some(crate::history::Exclusions::ro1()?),
            packing,
            |n, excluded, next| {
                raw += n;
                excluded_count += excluded;
                batches += 1;
                nonempty_batches += usize::from(n > excluded);
                cursor = next.clone();
                Ok(())
            },
        )?;
        packing_ok &= hit.is_none()
            && raw == 100
            && excluded_count == 90
            && batches == if packing { 3 } else { 25 }
            && nonempty_batches == if packing { 3 } else { 10 }
            && cursor.next().is_none();
    }
    report(
        "history packing reduces device batches and preserves completed cursor",
        packing_ok,
        &mut all_ok,
    );

    // Exercise transport of subtree events independently of coverage proofs:
    // one ordinary phrase, a synthetic covered block of nine, then a witness.
    // Mixed batches must retain the witness's original index even when no
    // per-phrase exclusion predicate is installed.
    fn subtree_steps(
        rows: &mut std::vec::IntoIter<[u16; 12]>,
        max_raw: usize,
    ) -> Option<crate::candidates::Step> {
        if rows.len() > 1 && rows.len() < 11 {
            let n = (rows.len() - 1).min(max_raw);
            rows.nth(n - 1)?;
            Some(crate::candidates::Step::Skipped(n))
        } else {
            rows.next().map(crate::candidates::Step::Candidate)
        }
    }
    let mut subtree_rows = vec![excluded; 10];
    subtree_rows.push(witness_ids);
    let mut pruning_ok = true;
    for (batch, pack_history) in [1, 4, 16]
        .into_iter()
        .flat_map(|batch| [false, true].map(|packing| (batch, packing)))
    {
        let mut completed_raw = 0;
        let mut completed_excluded = 0;
        let mut completed_pruned = 0;
        let hit = gpu.search_steps(
            subtree_rows.clone().into_iter(),
            &words,
            &witness_target,
            batch,
            64,
            11,
            true,
            None,
            true,
            pack_history,
            subtree_steps,
            |raw, excluded, pruned, _| {
                completed_raw += raw;
                completed_excluded += excluded;
                completed_pruned += pruned;
                Ok(())
            },
        )?;
        let expected_completed = if pack_history {
            usize::from(batch == 1)
        } else {
            (10 / batch) * batch
        };
        pruning_ok &= hit.is_some_and(|h| h.global_index == 10 && h.indices == witness_ids)
            && completed_raw == expected_completed
            && completed_excluded == expected_completed.saturating_sub(1)
            && completed_pruned == completed_excluded;
    }
    let mut completed_raw = 0;
    let mut completed_excluded = 0;
    let mut completed_pruned = 0;
    let mut completed_cursor = subtree_rows.clone().into_iter();
    let hit = gpu.search_steps(
        subtree_rows.clone().into_iter(),
        &words,
        &witness_target,
        4,
        64,
        10,
        true,
        None,
        true,
        true,
        subtree_steps,
        |raw, excluded, pruned, cursor| {
            completed_raw += raw;
            completed_excluded += excluded;
            completed_pruned += pruned;
            completed_cursor = cursor.clone();
            Ok(())
        },
    )?;
    pruning_ok &= hit.is_none()
        && completed_raw == 10
        && completed_excluded == 9
        && completed_pruned == 9
        && completed_cursor.next() == Some(witness_ids);
    let error = gpu.search_steps(
        subtree_rows.into_iter(),
        &words,
        &[0; 20],
        4,
        64,
        11,
        true,
        None,
        true,
        true,
        subtree_steps,
        |_, _, _, _| anyhow::bail!("intentional subtree checkpoint failure"),
    );
    pruning_ok &= error.is_err_and(|e| e.to_string().contains("intentional subtree"));
    report(
        "subtree steps, raw hit indices, limits, cursor and callback cleanup",
        pruning_ok,
        &mut all_ok,
    );
    let mut metrics = crate::metrics::SearchMetrics::default();
    let mut completed_raw = 0usize;
    let mut cursor_left = 50_000;
    let adaptive_hit = gpu.search_steps_observed(
        vec![[0u16; 12]; 50_000].into_iter(),
        &words,
        &[0; 20],
        65_536,
        64,
        50_000,
        true,
        None,
        false,
        true,
        true,
        None,
        &mut metrics,
        |it, _| it.next().map(crate::candidates::Step::Candidate),
        |raw, _, _, cursor| {
            completed_raw += raw;
            cursor_left = cursor.len();
            Ok(())
        },
    )?;
    let mut adaptive_ok = adaptive_hit.is_none()
        && completed_raw == 50_000
        && cursor_left == 0
        && metrics.completed_raw == 50_000
        && metrics.retained == 50_000
        && metrics.checksum_survivors == 0
        && metrics.gpu_seed_seconds == Some(0.0)
        && metrics.gpu_address_seconds == Some(0.0)
        && metrics.min_batch_size == Some(16_384)
        && metrics.adaptive_changes > 0
        && metrics.max_batch_size.is_some_and(|n| n <= 65_536);
    let mut rows = vec![[0u16; 12]; 2_000];
    rows.push(witness_ids);
    let mut hit_metrics = crate::metrics::SearchMetrics::default();
    let adaptive_hit = gpu.search_steps_observed(
        rows.into_iter(),
        &words,
        &witness_target,
        1024,
        64,
        2001,
        true,
        None,
        false,
        true,
        true,
        None,
        &mut hit_metrics,
        |it, _| it.next().map(crate::candidates::Step::Candidate),
        |_, _, _, _| Ok(()),
    )?;
    adaptive_ok &= adaptive_hit.is_some_and(|h| h.global_index == 2000 && h.indices == witness_ids)
        && hit_metrics.completed_raw == 1024
        && hit_metrics.completed_batches == 1
        && hit_metrics.checksum_survivors == 0
        && hit_metrics.gpu_seed_seconds == Some(0.0)
        && hit_metrics.gpu_address_seconds == Some(0.0);
    // Event timings count confirmed negative batches only, just like counts.
    // Dense valid input ensures both kernels run, independent of checksum luck.
    let mut timed_metrics = crate::metrics::SearchMetrics::default();
    let timed_hit = gpu.search_steps_observed(
        vec![witness_ids; 256].into_iter(),
        &words,
        &[0; 20],
        128,
        64,
        256,
        true,
        None,
        false,
        true,
        false,
        None,
        &mut timed_metrics,
        |it, _| it.next().map(crate::candidates::Step::Candidate),
        |_, _, _, _| Ok(()),
    )?;
    adaptive_ok &= timed_hit.is_none()
        && timed_metrics.completed_raw == 256
        && timed_metrics.checksum_survivors == 256
        && timed_metrics.gpu_seed_seconds.is_some_and(|s| s.is_finite() && s > 0.0)
        && timed_metrics.gpu_address_seconds.is_some_and(|s| s.is_finite() && s > 0.0);
    report(
        "adaptive batches, stage metrics, limits and uncommitted hit batch",
        adaptive_ok,
        &mut all_ok,
    );
    let stop_path = std::env::temp_dir().join(format!(
        "words-breaker-pause-{}-{}.signal",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut pause_metrics = crate::metrics::SearchMetrics::default();
    let mut remaining = 2048;
    let stopped = gpu.search_steps_observed(
        vec![[0u16; 12]; 2048].into_iter(),
        &words,
        &[0; 20],
        1024,
        64,
        2048,
        true,
        None,
        false,
        true,
        false,
        Some(stop_path.clone()),
        &mut pause_metrics,
        |it, _| it.next().map(crate::candidates::Step::Candidate),
        |_, _, _, cursor| {
            remaining = cursor.len();
            std::fs::write(&stop_path, b"pause")?;
            Ok(())
        },
    );
    let _ = std::fs::remove_file(&stop_path);
    let stopped = stopped?;
    report(
        "cooperative pause commits one batch and discards producer lookahead",
        stopped.is_none() && pause_metrics.completed_raw == 1024 && remaining == 1024,
        &mut all_ok,
    );
    Ok(all_ok)
}

/// Minimal SplitMix64 PRNG — deterministic test inputs without a rand dependency.
struct SplitMix64 {
    state: u64,
}
impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn fill32(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_be_bytes());
        }
        out
    }
}

fn report(name: &str, ok: bool, all_ok: &mut bool) {
    println!("  [{}] {}", if ok { "PASS" } else { "FAIL" }, name);
    *all_ok &= ok;
}

#[cfg(test)]
mod host_batch_tests {
    use super::*;
    use crate::candidates::Step;

    #[test]
    fn packing_caps_raw_work_and_preserves_retained_offsets() {
        let rule = crate::history::Exclusions::ro1().unwrap();
        let covered: [u16; 12] =
            "dutch update winter cattle fog lake also forest wood fiber fork parrot"
                .split_whitespace()
                .map(|w| {
                    bip39::Language::English
                        .word_list()
                        .binary_search(&w)
                        .unwrap() as u16
                })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
        let retained = [0; 12];
        assert!(rule.contains(&covered) && !rule.contains(&retained));
        let rows: Vec<_> = (0..100)
            .map(|i| if i % 10 == 9 { retained } else { covered })
            .collect();
        let mut buf = Vec::new();
        let mut positions = Vec::new();
        for packing in [false, true] {
            let mut iterator = rows.clone().into_iter();
            let batch = fill_host_batch(
                &mut iterator,
                &mut buf,
                &mut positions,
                4,
                usize::MAX,
                Some(&rule),
                true,
                packing,
                &mut |it, _| it.next().map(Step::Candidate),
                &mut || false,
            );
            assert_eq!(batch.raw, if packing { 40 } else { 4 });
            assert_eq!(buf.len() / 12, if packing { 4 } else { 0 });
            assert_eq!(
                positions,
                if packing { vec![9, 19, 29, 39] } else { vec![] }
            );
            assert!(!batch.exhausted);
        }
        let mut iterator = vec![covered; 300].into_iter();
        let first = fill_host_batch(
            &mut iterator,
            &mut buf,
            &mut positions,
            4,
            usize::MAX,
            Some(&rule),
            true,
            true,
            &mut |it, _| it.next().map(Step::Candidate),
            &mut || false,
        );
        assert_eq!(first.raw, 256);
        assert!(!first.exhausted && buf.is_empty());
        let second = fill_host_batch(
            &mut iterator,
            &mut buf,
            &mut positions,
            4,
            usize::MAX,
            Some(&rule),
            true,
            true,
            &mut |it, _| it.next().map(Step::Candidate),
            &mut || false,
        );
        assert_eq!(second.raw, 44);
        assert!(second.exhausted && buf.is_empty());
    }

    #[test]
    fn temporal_short_flush_does_not_mean_exhaustion_and_checks_large_skips() {
        let mut offset = 0usize;
        let mut buf = Vec::new();
        let mut positions = Vec::new();
        let mut source = |offset: &mut usize, max_raw: usize| {
            if *offset < 2048 {
                let n = (2048 - *offset).min(1024).min(max_raw);
                *offset += n;
                Some(Step::Skipped(n))
            } else if *offset < 2050 {
                *offset += 1;
                Some(Step::Candidate([0; 12]))
            } else {
                None
            }
        };
        for _ in 0..2 {
            let mut time_checks = 0;
            let batch = fill_host_batch(
                &mut offset,
                &mut buf,
                &mut positions,
                2048,
                usize::MAX,
                None,
                true,
                true,
                &mut source,
                &mut || {
                    time_checks += 1;
                    true
                },
            );
            assert_eq!((batch.raw, batch.pruned, time_checks), (1024, 1024, 1));
            assert!(!batch.exhausted && buf.is_empty());
        }
        let final_batch = fill_host_batch(
            &mut offset,
            &mut buf,
            &mut positions,
            2048,
            usize::MAX,
            None,
            true,
            true,
            &mut source,
            &mut || false,
        );
        assert_eq!((final_batch.raw, final_batch.pruned), (2, 0));
        assert!(final_batch.exhausted);
        assert_eq!(positions, vec![0, 1]);
        assert_eq!(offset, 2050);
    }

    #[test]
    fn unfiltered_batches_keep_raw_behavior_and_zero_limit_does_not_advance() {
        let mut iterator = vec![[0; 12]; 20].into_iter();
        let mut buf = Vec::new();
        let mut positions = Vec::new();
        for left in [0, 3, usize::MAX] {
            let before = iterator.len();
            let batch = fill_host_batch(
                &mut iterator,
                &mut buf,
                &mut positions,
                4,
                left,
                None,
                false,
                true,
                &mut |it, _| it.next().map(Step::Candidate),
                &mut || panic!("unfiltered batching must not consult packing deadline"),
            );
            assert_eq!(batch.raw, left.min(4));
            assert_eq!(iterator.len(), before - batch.raw);
            assert_eq!(buf.len() / 12, batch.raw);
            assert!(positions.is_empty());
        }
    }
}
