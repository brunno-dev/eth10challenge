// Route the existing diagnostics through JSONL when serving a resident worker.
// Keeping these before the modules also captures logs from producer threads.
macro_rules! println {
    ($($arg:tt)*) => { crate::output::line(false, format_args!($($arg)*)) };
}
#[allow(unused_macros)] // Non-CUDA release builds currently have no stderr logs.
macro_rules! eprintln {
    ($($arg:tt)*) => { crate::output::line(true, format_args!($($arg)*)) };
}

use anyhow::{Context, Result};
use bip39::{Language, Mnemonic};
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1::Secp256k1;
use clap::Parser;
use rayon::prelude::*;
use std::time::Instant;

#[cfg(any(feature = "cuda", test))]
mod adaptive;
mod candidates;
mod checkpoint;
mod eth;
#[cfg(feature = "cuda")]
mod gpu;
mod history;
mod local_history;
mod metrics;
mod output;
mod plan;
#[cfg(test)]
mod plan_union_tests;
mod pruning;
#[cfg(test)]
mod pruning_cursor_tests;
mod worker;
use unicode_normalization::UnicodeNormalization;

use candidates::Slot;
use plan::Plan;

#[derive(Parser, Debug)]
#[command(
    about = "Try permutations of BIP-39 words (10-12) to match an Ethereum address \
(BIP-44 m/44'/60'/0'/0/0). Missing words (when 10 or 11 are given) are filled from the \
2048-word BIP-39 list.",
    version
)]
struct Args {
    /// Target Ethereum address, e.g. 0x9858EfFD232B4033E47d90003D41EC34EcaEda94.
    /// Mixed-case input is checked against its EIP-55 checksum.
    /// Optional only when --selftest is given.
    target_address: Option<String>,

    /// 10, 11, or 12 words (unordered or partially ordered). Missing words are
    /// completed from the BIP-39 wordlist. Ignored when --pattern is given.
    words: Vec<String>,

    /// Positional template: 12 slots separated by spaces, `?` for a slot whose
    /// word is unknown. Words at the other slots are pinned there.
    /// Example: --pattern "dutch ? ? ? fog ? ? ? ? ? ? parrot"
    #[arg(long)]
    pattern: Option<String>,

    /// Words that fill the `?` slots of --pattern, each used at most once
    /// (space- or comma-separated). If there are more `?` slots than pool words,
    /// each leftover slot is drawn from the full BIP-39 list, which multiplies
    /// the search space by 2048 per slot.
    #[arg(long)]
    pool: Option<String>,

    /// Restricts what a `?` slot may hold once the pool is spent. Accepts
    /// literal words and `prefix*` patterns, space- or comma-separated.
    /// Defaults to the entire wordlist. Each unfilled slot multiplies the search
    /// by this set's size, so narrowing it is the strongest lever available:
    /// --fill "d*,f*" cuts 2048 down to 218.
    #[arg(long)]
    fill: Option<String>,

    /// Six words from the post. word@N pins a word (1-based); bare words are
    /// candidates for the remaining post slots, drawn without replacement.
    #[arg(long, conflicts_with_all = ["pattern", "pool", "fill", "words"], requires = "video")]
    post: Option<String>,

    /// Six words from the video, using the same word@N syntax as --post.
    #[arg(long, requires = "post")]
    video: Option<String>,

    /// Derive every phrase, even with an invalid BIP-39 checksum (~16x more
    /// derivations). For phrases produced by a non-conforming generator.
    #[arg(long)]
    no_checksum: bool,

    /// BIP-39 wordlist language (english, portuguese, spanish, french, italian, czech, korean, japanese, chinese-simplified, chinese-traditional)
    #[arg(long, short, default_value = "english")]
    language: String,

    /// Number of threads to use (defaults to number of CPU cores)
    #[arg(long, short, default_value_t = 0)]
    threads: usize,

    /// Verify each GPU crypto primitive against the CPU reference and exit.
    #[arg(long)]
    selftest: bool,

    /// Force the CPU (rayon) search instead of the GPU. The GPU is used by
    /// default when a CUDA device is available.
    #[arg(long)]
    cpu: bool,

    /// GPU candidates per batch (CPU caps this at 4096).
    #[arg(long, default_value_t = 1 << 20, value_parser = parse_batch_size)]
    batch_size: usize,

    /// GPU threads per block: compare throughput for 64, 128 and 256.
    #[arg(long, default_value_t = 64, value_parser = parse_block_size)]
    block_size: u32,

    /// Stop after this many additional candidates, without claiming exhaustion.
    #[arg(long, value_parser = parse_positive)]
    max_candidates: Option<usize>,

    /// Save completed progress every ten seconds and at normal termination.
    #[arg(long)]
    checkpoint: Option<std::path::PathBuf>,

    /// Continue a checkpoint with identical search inputs, without replaying them.
    #[arg(long, requires = "checkpoint")]
    resume: bool,

    /// Print the versioned external search catalog and exit (no target needed).
    #[arg(long, conflicts_with_all = ["coverage_report", "exclude_tested", "exclude_record", "record_search", "selftest"])]
    list_history: bool,

    /// Inspect selected history overlap (RO1 by default) without derivations. First 1M candidates;
    /// --max-candidates changes the bound. Does not write a checkpoint.
    #[arg(long, conflicts_with_all = ["checkpoint", "resume", "record_search", "selftest"])]
    coverage_report: bool,

    /// Trust the archived external negative and skip its exact RO1 domain.
    /// Requires the challenge target, English, default path and checksum checking.
    #[arg(long, value_parser = ["RO1"], conflicts_with = "selftest")]
    exclude_tested: Option<String>,

    /// Reuse a complete local negative record. Repeat this option for multiple files.
    #[arg(long, conflicts_with = "selftest")]
    exclude_record: Vec<std::path::PathBuf>,

    /// Reuse every compatible negative record in this directory.
    #[arg(long, conflicts_with = "selftest")]
    exclude_record_dir: Option<std::path::PathBuf>,

    /// Additional compatible negatives on resume; does not change the original checkpoint inputs.
    #[arg(long, requires = "resume", conflicts_with_all = ["selftest", "export_checkpoint_record"])]
    additional_record_dir: Option<std::path::PathBuf>,

    /// Inspect full historical coverage without starting a search.
    #[arg(long, conflicts_with_all = ["selftest", "list_history", "coverage_report", "checkpoint", "resume", "record_search", "record_progress", "export_checkpoint_record"])]
    preflight_history: bool,

    /// Save the confirmed negative prefix after normal termination, replacing this record.
    #[arg(long, conflicts_with_all = ["selftest", "list_history", "coverage_report", "record_search", "export_checkpoint_record"])]
    record_progress: Option<std::path::PathBuf>,

    /// Export a checkpoint whose negative outcome has been confirmed by the caller.
    #[arg(long, requires = "checkpoint", conflicts_with_all = ["selftest", "list_history", "coverage_report", "resume", "record_search", "record_progress"])]
    export_checkpoint_record: Option<std::path::PathBuf>,

    /// Disable covered-prefix pruning for comparison; history filtering still applies.
    #[arg(long)]
    no_prune_history: bool,

    /// Keep GPU batches bounded by raw candidates instead of accumulating survivors.
    #[arg(long)]
    no_pack_history: bool,

    /// Recompute subtree counts instead of reusing the bounded in-memory cache.
    #[arg(long)]
    no_count_cache: bool,

    /// Adapt GPU batches to observed latency; --batch-size remains the ceiling.
    #[arg(long)]
    adaptive_batch: bool,

    /// Print completed-work counters and host timing by processing stage.
    #[arg(long, conflicts_with_all = ["list_history", "coverage_report", "selftest"])]
    metrics: bool,

    /// Write completed-work metrics as JSON to a new file at normal termination.
    #[arg(long, conflicts_with_all = ["list_history", "coverage_report", "selftest"])]
    metrics_json: Option<std::path::PathBuf>,

    /// Desktop control: finish the current batch when this file appears.
    #[arg(long, hide = true)]
    stop_file: Option<std::path::PathBuf>,

    /// Write a reusable negative record only after complete exhaustion without exclusions.
    #[arg(long, conflicts_with_all = ["exclude_tested", "exclude_record", "selftest"])]
    record_search: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    if std::env::args_os()
        .skip(1)
        .eq([std::ffi::OsStr::new("--worker")])
    {
        if worker::run().is_err() {
            // Broken protocol I/O is reported by the caller as process loss;
            // do not let Rust's Result termination print an unframed error.
            std::process::exit(1);
        }
        return Ok(());
    }
    let args = Args::parse();
    execute(args, &mut Session::default())
}

/// A session is owned and used by the same OS thread throughout its lifetime.
/// CUDA contexts, loaded kernels and the fixed-base table survive between jobs.
#[derive(Default)]
struct Session {
    #[cfg(feature = "cuda")]
    gpu: Option<gpu::Gpu>,
    #[cfg(feature = "cuda")]
    gpu_initialization_attempted: bool,
    fatal: bool,
}

fn execute(args: Args, session: &mut Session) -> Result<()> {
    anyhow::ensure!(
        !session.fatal,
        "The search session cannot be reused after a GPU failure"
    );

    if args.list_history {
        println!("{}", history::CATALOG);
        return Ok(());
    }

    if args.selftest {
        #[cfg(feature = "cuda")]
        {
            anyhow::ensure!(gpu::run_selftest()?, "One or more GPU selftests failed");
            println!("All GPU selftests passed.");
            return Ok(());
        }
        #[cfg(not(feature = "cuda"))]
        anyhow::bail!("--selftest requires a build with the cuda feature");
    }

    let target_address = args
        .target_address
        .as_deref()
        .context("Missing target address")?;

    let target = eth::parse_address(target_address)?;
    let language = parse_language(&args.language)?;
    let wordlist: &'static [&'static str] = language.words_by_prefix("");

    let plan = build_plan(&args, wordlist)?;
    for output in [
        args.record_progress.as_ref(),
        args.export_checkpoint_record.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        normalized_output_path(output)?;
        for input in [args.checkpoint.as_ref(), args.metrics_json.as_ref()]
            .into_iter()
            .flatten()
            .chain(args.exclude_record.iter())
        {
            anyhow::ensure!(
                !same_output_path(output, input)?,
                "history output must differ from checkpoint, metrics and input records"
            );
        }
    }
    if let Some(path) = &args.metrics_json {
        anyhow::ensure!(
            !path.exists(),
            "metrics path already exists; choose a new path"
        );
        normalized_output_path(path)?;
        for other in [args.checkpoint.as_ref(), args.record_search.as_ref()]
            .into_iter()
            .flatten()
        {
            anyhow::ensure!(
                !same_output_path(path, other)?,
                "metrics path must differ from checkpoint and record paths"
            );
        }
    }
    let slots = *plan.slots();
    let pool = plan.report_pool();

    println!("Target: {} ({})", eth::to_eip55(&target), eth::ETH_PATH);
    plan.describe(wordlist)?;
    if let Some(path) = &args.record_search {
        anyhow::ensure!(
            !path.exists(),
            "record path already exists; choose a new path"
        );
        anyhow::ensure!(
            args.checkpoint.as_ref() != Some(path),
            "record and checkpoint paths must differ"
        );
    }
    if args.no_checksum {
        println!(
            "BIP-39 checksum filter disabled: deriving every candidate (~16x more derivations)"
        );
    }
    let record_paths = compatible_record_paths(&args, language, &target)?;
    let exclusion =
        if args.exclude_tested.is_some() || !record_paths.is_empty() || args.coverage_report {
            let ro1 =
                args.exclude_tested.is_some() || (args.coverage_report && record_paths.is_empty());
            let rules = history::Exclusions::load(ro1, &record_paths)?;
            rules.compatible(language, &target, args.no_checksum)?;
            println!("History selection: {}", rules.fingerprint()?);
            if ro1 {
                println!("RO1 uses an externally reported negative.");
            }
            Some(rules)
        } else {
            None
        };
    if let Some(output) = &args.export_checkpoint_record {
        let record = checkpoint::export_record(
            &plan,
            &args.language,
            target,
            args.no_checksum,
            args.checkpoint.as_ref().unwrap(),
            exclusion.as_ref(),
        )?;
        record.save_replace(output)?;
        println!(
            "Checkpoint negative coverage recorded: {}",
            output.display()
        );
        return Ok(());
    }
    if args.preflight_history {
        let total = plan.count()?;
        let covered = history_covers(&plan, exclusion.as_ref())?;
        println!(
            "History preflight: {}",
            serde_json::json!({"fullyCovered": covered, "total":total.to_string(), "records":record_paths.len()})
        );
        return Ok(());
    }
    if args.coverage_report {
        return history::report(
            &plan,
            args.max_candidates.unwrap_or(1_000_000),
            exclusion.as_ref().unwrap(),
        );
    }
    // Exact whole-domain proof, including reordered pools. Checkpointed searches
    // still advance normal cursors so raw limits and resumption remain unchanged.
    if args.checkpoint.is_none() && exclusion.as_ref().is_some_and(|r| r.covers_plan(&plan)) {
        println!("Entire domain already covered: {} candidates skipped without enumeration or GPU initialization", plan.count()?);
        let metrics = metrics::SearchMetrics {
            backend: "coverage-proof".into(),
            ..Default::default()
        };
        if args.metrics {
            metrics.print();
        }
        if let Some(path) = &args.metrics_json {
            metrics.write_new(path)?;
        }
        return Ok(());
    }
    let mut progress = checkpoint::Progress::with_history(
        &plan,
        &args.language,
        target,
        args.no_checksum,
        args.checkpoint.clone(),
        args.resume,
        exclusion,
    )?;
    if let Some(directory) = &args.additional_record_dir {
        let mut combined = record_paths.clone();
        append_compatible_records(
            directory,
            language,
            &target,
            args.no_checksum,
            &mut combined,
        )?;
        if !combined.is_empty() || args.exclude_tested.is_some() {
            let rules = history::Exclusions::load(args.exclude_tested.is_some(), &combined)?;
            rules.compatible(language, &target, args.no_checksum)?;
            // The original fingerprint was checked above and remains in Config.
            // Extra proven negatives cannot change enumeration or invalidate its cursor.
            progress.exclusion = Some(rules);
            println!("Additional compatible history loaded for resumed search");
        }
    }
    let already_covered = history_covers(&plan, progress.exclusion.as_ref())?;
    let initial = progress.checked;
    progress.candidates.set_count_cache(!args.no_count_cache);
    let initial_excluded = progress.excluded;
    if args.resume {
        println!("Resumed after {} completed candidates", progress.checked);
    }
    let start = Instant::now();
    if !args.no_prune_history {
        if let Some(rule) = &progress.exclusion {
            let pruning = pruning::Pruning::compile(&plan, rule);
            if !pruning.is_empty() {
                progress.pruning = Some(std::sync::Arc::new(pruning));
            }
        }
    }
    let found;
    #[cfg(feature = "cuda")]
    {
        // Only initialization failure falls back. A failure during a search is
        // propagated, leaving its checkpoint available instead of restarting.
        if !args.cpu && !already_covered && !session.gpu_initialization_attempted {
            session.gpu_initialization_attempted = true;
            match gpu::Gpu::new() {
                Ok(gpu) => {
                    session.gpu = Some(gpu);
                    println!("CUDA context initialized");
                }
                Err(e) => {
                    eprintln!("CUDA initialization unavailable ({e:#}); using CPU");
                }
            }
        } else if !args.cpu && !already_covered && session.gpu.is_some() {
            println!("Reusing initialized CUDA context");
        }
        let device = if args.cpu || already_covered {
            None
        } else {
            session.gpu.as_ref()
        };
        found = if let Some(gpu) = device {
            let result = search_gpu(
                gpu,
                &args,
                &mut progress,
                &slots,
                &pool,
                wordlist,
                language,
                &target,
            );
            // A search error can leave pending launches or a poisoned CUDA
            // context. Never run another job or retry the failed one here.
            if result.is_err() {
                session.fatal = true;
                session.gpu.take();
            }
            result?
        } else {
            run_cpu_search(
                &args,
                &mut progress,
                &slots,
                &pool,
                wordlist,
                language,
                &target,
            )?
        };
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = already_covered;
        found = run_cpu_search(
            &args,
            &mut progress,
            &slots,
            &pool,
            wordlist,
            language,
            &target,
        )?;
    }
    let save_start = Instant::now();
    progress.save()?;
    progress.metrics.checkpoint_seconds += save_start.elapsed().as_secs_f64();
    if progress.exclusion.is_some() {
        println!("History excluded {} arrangements this run ({} cumulative); limits and cursors count original candidates", progress.excluded - initial_excluded, progress.excluded);
    }
    if progress.pruning.is_some() {
        println!(
            "Pruned {} candidates without generation this run",
            progress.pruned_this_run
        );
    }
    if !found {
        let exhausted = progress.candidates.clone().next().is_none();
        if let Some(path) = &args.record_progress {
            if progress.checked > 0 {
                local_history::Record::confirmed(
                    &plan,
                    &args.language,
                    target,
                    args.no_checksum,
                    progress.checked,
                    progress.candidates.clone().next(),
                )?
                .save_replace(path)?;
                println!("Confirmed negative coverage recorded: {}", path.display());
            }
        }
        if let Some(path) = &args.record_search {
            if exhausted {
                local_history::Record::completed(
                    &plan,
                    &args.language,
                    target,
                    args.no_checksum,
                    &progress,
                )?
                .save(path)?;
                println!("Complete local negative recorded: {}", path.display());
            } else {
                println!("Local record not created: search is incomplete; resume its checkpoint to finish");
            }
        }
        let checked = progress.checked - initial;
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "{}: {} candidates in {:.3}s ({:.0} candidates/s)",
            if exhausted {
                "Exhausted search without a match"
            } else if args.stop_file.as_ref().is_some_and(|path| path.exists()) {
                "Paused search; checkpoint saved"
            } else {
                "Candidate limit reached; search incomplete"
            },
            checked,
            seconds,
            checked as f64 / seconds.max(1e-9)
        );
    }
    if args.metrics {
        progress.metrics.print();
    }
    if let Some(path) = &args.metrics_json {
        progress.metrics.write_new(path)?;
    }
    Ok(())
}

fn compatible_record_paths(
    args: &Args,
    language: Language,
    target: &[u8; 20],
) -> Result<Vec<std::path::PathBuf>> {
    let mut paths = args.exclude_record.clone();
    if let Some(directory) = &args.exclude_record_dir {
        append_compatible_records(directory, language, target, args.no_checksum, &mut paths)?;
    }
    Ok(paths)
}

fn append_compatible_records(
    directory: &std::path::Path,
    language: Language,
    target: &[u8; 20],
    no_checksum: bool,
    paths: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| format!("reading history directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if !entry.file_type()?.is_file()
            || !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let record = local_history::Record::load(&path)
            .with_context(|| format!("invalid automatic history record {}", path.display()))?;
        if record.is_compatible(language, target, no_checksum) {
            paths.push(path);
        }
    }
    Ok(())
}

fn history_covers(plan: &Plan, rules: Option<&history::Exclusions>) -> Result<bool> {
    let Some(rules) = rules else {
        return Ok(false);
    };
    if rules.covers_plan(plan) {
        return Ok(true);
    }
    // Exact bounded fallback for unions of partial prefixes, without derivations.
    Ok(plan.count()? <= 1_000_000 && plan.stream().all(|phrase| rules.contains(&phrase)))
}

fn normalized_output_path(path: &std::path::Path) -> Result<std::path::PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let name = path.file_name().context("output path must name a file")?;
    Ok(std::fs::canonicalize(parent)
        .with_context(|| format!("resolving output directory {}", parent.display()))?
        .join(name))
}

fn same_output_path(a: &std::path::Path, b: &std::path::Path) -> Result<bool> {
    let a = normalized_output_path(a)?;
    let b = normalized_output_path(b)?;
    #[cfg(windows)]
    {
        Ok(a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()))
    }
    #[cfg(not(windows))]
    {
        Ok(a == b)
    }
}

fn parse_positive(s: &str) -> Result<usize, String> {
    let n = s
        .parse::<usize>()
        .map_err(|_| "expected a positive integer".to_string())?;
    if n == 0 {
        return Err("must be greater than zero".into());
    }
    Ok(n)
}
fn parse_batch_size(s: &str) -> Result<usize, String> {
    let n = parse_positive(s)?;
    if n > (u32::MAX / 12) as usize {
        return Err("batch too large".into());
    }
    Ok(n)
}
fn parse_block_size(s: &str) -> Result<u32, String> {
    let n = s
        .parse::<u32>()
        .map_err(|_| "expected 32, 64, 128 or 256".to_string())?;
    if ![32, 64, 128, 256].contains(&n) {
        return Err("expected 32, 64, 128 or 256".into());
    }
    Ok(n)
}

/// Resolves the CLI into a 12-slot template plus the pool that fills its holes.
///
/// `--pattern`/`--pool` build it directly. Loose positional words are the same
/// thing with every slot open, so both modes go through one enumerator.
fn build_plan(args: &Args, wordlist: &[&str]) -> Result<Plan> {
    if let (Some(post_spec), Some(video_spec)) = (args.post.as_deref(), args.video.as_deref()) {
        let mut slots = [Slot::Hole; 12];
        let index_of = |word: &str| {
            let word: String = word.nfkd().collect();
            wordlist
                .iter()
                .position(|w| *w == word)
                .map(|i| i as u16)
                .with_context(|| {
                    format!("'{word}' is not in the {} BIP-39 wordlist", args.language)
                })
        };
        let mut parse_batch = |spec: &str, name: &str| -> Result<(Vec<u16>, usize)> {
            let mut pool = Vec::new();
            let mut pins = 0;
            for token in spec
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
            {
                if let Some((word, position)) = token.split_once('@') {
                    let position = position
                        .parse::<usize>()
                        .ok()
                        .filter(|p| (1..=12).contains(p))
                        .with_context(|| format!("--{name}: '{token}' — position must be 1-12"))?;
                    anyhow::ensure!(
                        slots[position - 1] == Slot::Hole,
                        "--{name}: position {position} is pinned twice"
                    );
                    slots[position - 1] = Slot::Fixed(index_of(word)?);
                    pins += 1;
                } else {
                    pool.push(index_of(token)?);
                }
            }
            anyhow::ensure!(
                pins <= 6,
                "--{name} pins {pins} positions; each batch holds only 6 words"
            );
            let open = 6 - pins;
            anyhow::ensure!(
                pool.len() >= open,
                "--{name} requires at least {open} unpinned candidates, got {}",
                pool.len()
            );
            Ok((pool, open))
        };
        let (post, post_open) = parse_batch(post_spec, "post")?;
        let (video, _) = parse_batch(video_spec, "video")?;
        Ok(Plan::Batches {
            slots,
            post,
            video,
            post_open,
        })
    } else {
        let (slots, pool, fill) = build_template(args, wordlist)?;
        Ok(Plan::Template { slots, pool, fill })
    }
}

fn build_template(args: &Args, wordlist: &[&str]) -> Result<([Slot; 12], Vec<u16>, Vec<u16>)> {
    let index_of = |w: &str| -> Result<u16> {
        let normalized: String = w.nfkd().collect();
        wordlist
            .iter()
            .position(|x| *x == normalized)
            .map(|p| p as u16)
            .with_context(|| format!("'{w}' is not in the {} BIP-39 wordlist", args.language))
    };

    // Which words a `?` slot may take once the pool runs out.
    let fill: Vec<u16> = match args.fill.as_deref() {
        None => (0..wordlist.len() as u16).collect(),
        Some(spec) => {
            let mut set: Vec<u16> = Vec::new();
            for tok in spec.split([' ', ',', '\t', '\n']).filter(|s| !s.is_empty()) {
                match tok.strip_suffix('*') {
                    // `abc*` — every word with that prefix.
                    Some(prefix) => {
                        let prefix: String = prefix.nfkd().collect();
                        let before = set.len();
                        set.extend(
                            wordlist
                                .iter()
                                .enumerate()
                                .filter(|(_, w)| w.starts_with(&prefix))
                                .map(|(i, _)| i as u16),
                        );
                        anyhow::ensure!(
                            set.len() > before,
                            "no {} BIP-39 word starts with '{prefix}'",
                            args.language
                        );
                    }
                    None => set.push(index_of(tok)?),
                }
            }
            set.sort_unstable();
            set.dedup();
            anyhow::ensure!(!set.is_empty(), "--fill matched no words");
            set
        }
    };

    let Some(pattern) = args.pattern.as_deref() else {
        anyhow::ensure!(
            args.pool.is_none(),
            "--pool only means something with --pattern; without it, pass the words positionally"
        );
        anyhow::ensure!(
            (10..=12).contains(&args.words.len()),
            "Expected 10, 11, or 12 words, got {}",
            args.words.len()
        );
        let pool = args
            .words
            .iter()
            .map(|w| index_of(w))
            .collect::<Result<Vec<u16>>>()?;
        return Ok(([Slot::Hole; 12], pool, fill));
    };

    let fields: Vec<&str> = pattern.split_whitespace().collect();
    anyhow::ensure!(
        fields.len() == 12,
        "--pattern needs exactly 12 slots, got {} in {pattern:?}",
        fields.len()
    );
    let mut slots = [Slot::Hole; 12];
    for (i, f) in fields.iter().enumerate() {
        if *f != "?" {
            slots[i] = Slot::Fixed(index_of(f)?);
        }
    }

    let pool = args
        .pool
        .as_deref()
        .unwrap_or("")
        .split([' ', ',', '\t', '\n'])
        .filter(|s| !s.is_empty())
        .map(index_of)
        .collect::<Result<Vec<u16>>>()?;

    Ok((slots, pool, fill))
}

/// Prints the shape of the space before committing to a long run — a miscounted
/// pool or a stray `?` is much cheaper to notice here than an hour in.
fn describe_search(
    slots: &[Slot; 12],
    pool: &[u16],
    fill: &[u16],
    wordlist: &[&str],
) -> Result<()> {
    let holes = slots.iter().filter(|s| **s == Slot::Hole).count();
    let fixed = 12 - holes;
    let total = candidates::count(slots, pool, fill)
        .context("candidate count exceeds u128; narrow the search")?;

    println!(
        "Template: {} pinned, {} open slot(s), {} pool word(s)",
        fixed,
        holes,
        pool.len()
    );
    if pool.len() < holes {
        let free = holes - pool.len();
        let scope = if fill.len() == wordlist.len() {
            format!("the full {}-word list", wordlist.len())
        } else {
            format!("a restricted set of {} word(s)", fill.len())
        };
        println!(
            "  {free} slot(s) have no pool word and will be drawn from {scope} (x{} per slot)",
            fill.len()
        );
    } else if pool.len() > holes {
        println!(
            "  pool has {} more word(s) than slots, so subsets are enumerated too",
            pool.len() - holes
        );
    }
    println!(
        "Searching {} distinct candidates ({} exact; streamed)...",
        format_u128(total),
        total
    );
    Ok(())
}

/// Run independent batches in a local pool: no per-candidate bridge or atomics.
fn run_cpu_search(
    args: &Args,
    progress: &mut checkpoint::Progress,
    slots: &[Slot; 12],
    pool: &[u16],
    wordlist: &[&str],
    language: Language,
    target: &[u8; 20],
) -> Result<bool> {
    let threads = if args.threads == 0 {
        num_cpus::get()
    } else {
        args.threads
    };
    let workers = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()?;
    println!("Using CPU with {threads} threads");
    progress.metrics.backend = "CPU".into();
    let secp = Secp256k1::new();
    let path: DerivationPath = eth::ETH_PATH.parse()?;
    let mut it = progress.candidates.clone();
    let mut left = args.max_candidates.unwrap_or(usize::MAX);
    let mut batch = Vec::with_capacity(args.batch_size.min(4096));
    let mut last_print = Instant::now();
    while left > 0 && !args.stop_file.as_ref().is_some_and(|path| path.exists()) {
        let producer_start = Instant::now();
        batch.clear();
        let capacity = left.min(args.batch_size).min(4096);
        let mut original_len = 0;
        let mut pruned = 0;
        if let Some(pruning) = &progress.pruning {
            while original_len < capacity {
                match it.next_pruned(capacity - original_len, pruning.as_ref()) {
                    Some(candidates::Step::Candidate(phrase)) => {
                        batch.push(phrase);
                        original_len += 1;
                    }
                    Some(candidates::Step::Skipped(n)) => {
                        original_len += n;
                        pruned += n;
                    }
                    None => break,
                }
            }
        } else {
            batch.extend(it.by_ref().take(capacity));
            original_len = batch.len();
        }
        if original_len == 0 {
            break;
        }
        if let Some(rule) = &progress.exclusion {
            batch.retain(|phrase| !rule.contains(phrase));
        }
        let producer_seconds = producer_start.elapsed().as_secs_f64();
        let survivors = std::sync::atomic::AtomicUsize::new(0);
        let derive_start = Instant::now();
        let hit = workers
            .install(|| {
                batch
                    .par_iter()
                    .map(|indices| -> Result<Option<[u16; 12]>> {
                        let Some(mnemonic) =
                            candidate_mnemonic(indices, language, args.no_checksum)
                        else {
                            return Ok(None);
                        };
                        survivors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let seed = mnemonic.to_seed_normalized("");
                        let addr = eth::address_from_seed(&secp, &path, &seed)?;
                        Ok((addr == *target).then_some(*indices))
                    })
                    .find_any(|result| !matches!(result, Ok(None)))
            })
            .transpose()?
            .flatten();
        if let Some(indices) = hit {
            report_hit(&indices, None, slots, pool, wordlist, target);
            return Ok(true);
        }
        let derive_seconds = derive_start.elapsed().as_secs_f64();
        let checkpoint_start = Instant::now();
        progress.completed_pruned(original_len, original_len - batch.len(), pruned, &it)?;
        progress.metrics.checkpoint_seconds += checkpoint_start.elapsed().as_secs_f64();
        progress
            .metrics
            .record_batch(original_len, batch.len(), pruned, survivors.into_inner())?;
        progress.metrics.observe_batch_size(capacity);
        progress.metrics.producer_seconds += producer_seconds;
        progress.metrics.derive_seconds += derive_seconds;
        left -= original_len;
        if last_print.elapsed().as_secs() >= 2 {
            println!("Checked {} distinct candidates", progress.checked);
            last_print = Instant::now();
        }
    }
    Ok(false)
}

#[cfg(feature = "cuda")]
#[allow(clippy::too_many_arguments)]
fn search_gpu(
    gpu: &gpu::Gpu,
    args: &Args,
    progress: &mut checkpoint::Progress,
    slots: &[Slot; 12],
    pool: &[u16],
    wordlist: &[&str],
    language: Language,
    target: &[u8; 20],
) -> Result<bool> {
    // Without the 1/16 filter, a default batch would perform sixteen times as
    // much work per launch. Keep launch duration comparable on display GPUs.
    let batch_size = if args.no_checksum {
        args.batch_size.min(1 << 16)
    } else {
        args.batch_size
    };
    println!(
        "Using GPU (CUDA), batch={}, block={}",
        batch_size, args.block_size
    );
    let gpu_wordlist = gpu::GpuWordlist::new(wordlist)?;
    let initial = progress.checked;
    let mut metrics = metrics::SearchMetrics {
        backend: "CUDA".into(),
        ..Default::default()
    };
    let result = gpu.search_observed(
        progress.candidates.clone(),
        &gpu_wordlist,
        target,
        batch_size,
        args.block_size,
        args.max_candidates.unwrap_or(usize::MAX),
        !args.no_checksum,
        progress.exclusion.clone(),
        progress.pruning.clone(),
        !args.no_pack_history,
        args.adaptive_batch,
        args.stop_file.clone(),
        &mut metrics,
        |n, excluded, pruned, cursor| progress.completed_pruned(n, excluded, pruned, cursor),
    );
    progress.metrics = metrics;
    let hit = result?;
    if let Some(h) = hit {
        let mnemonic = candidate_mnemonic(&h.indices, language, args.no_checksum)
            .context("GPU hit has invalid words or checksum")?;
        let seed = mnemonic.to_seed_normalized("");
        let actual = eth::address_from_seed(&Secp256k1::new(), &eth::ETH_PATH.parse()?, &seed)?;
        anyhow::ensure!(
            actual == *target,
            "GPU result failed independent CPU verification"
        );
        report_hit(
            &h.indices,
            Some(initial + h.global_index as u128),
            slots,
            pool,
            wordlist,
            &actual,
        );
        return Ok(true);
    }
    Ok(false)
}

fn candidate_mnemonic(
    indices: &[u16; 12],
    language: Language,
    no_checksum: bool,
) -> Option<Mnemonic> {
    if no_checksum {
        // Do not reconstruct entropy here: that would repair the checksum and
        // silently derive a different last word from the candidate being tried.
        let words = language.word_list();
        let phrase = indices
            .iter()
            .map(|&i| words.get(i as usize).copied())
            .collect::<Option<Vec<_>>>()?
            .join(" ");
        Mnemonic::parse_in_normalized_without_checksum_check(language, &phrase).ok()
    } else {
        let entropy = eth::checked_entropy(indices)?;
        Mnemonic::from_entropy_in(language, &entropy).ok()
    }
}

/// Prints a match, calling out any word that came from the full wordlist rather
/// than from the pattern or the pool — those are the genuinely recovered ones.
fn report_hit(
    indices: &[u16; 12],
    index: Option<u128>,
    slots: &[Slot; 12],
    pool: &[u16],
    wordlist: &[&str],
    target: &[u8; 20],
) {
    let phrase: Vec<&str> = indices.iter().map(|&i| wordlist[i as usize]).collect();
    println!("Found matching mnemonic: {}", phrase.join(" "));

    let mut unused: Vec<u16> = pool.to_vec();
    let mut recovered: Vec<String> = Vec::new();
    for (i, &w) in indices.iter().enumerate() {
        if slots[i] != Slot::Hole {
            continue; // pinned by --pattern
        }
        match unused.iter().position(|&p| p == w) {
            Some(p) => {
                unused.remove(p);
            }
            None => recovered.push(format!("position {}: {}", i + 1, wordlist[w as usize])),
        }
    }
    if !recovered.is_empty() {
        println!("Recovered from the full wordlist: {}", recovered.join(", "));
    }
    if let Some(i) = index {
        println!("Candidate index (0-based): {}", i);
    }
    println!("Derived address: {}", eth::to_eip55(target));
}

pub fn format_number(n: usize) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}G", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn parse_language(lang: &str) -> Result<Language> {
    match lang.to_lowercase().as_str() {
        "english" => Ok(Language::English),
        "portuguese" => Ok(Language::Portuguese),
        "spanish" => Ok(Language::Spanish),
        "french" => Ok(Language::French),
        "italian" => Ok(Language::Italian),
        "czech" => Ok(Language::Czech),
        "korean" => Ok(Language::Korean),
        "japanese" => Ok(Language::Japanese),
        "chinese-simplified" => Ok(Language::SimplifiedChinese),
        "chinese-traditional" => Ok(Language::TraditionalChinese),
        _ => anyhow::bail!("Unknown language: {}. Supported: english, portuguese, spanish, french, italian, czech, korean, japanese, chinese-simplified, chinese-traditional", lang),
    }
}

/// Formats a `u128` candidate count with a magnitude suffix.
fn format_u128(n: u128) -> String {
    const UNITS: [(u128, &str); 4] = [
        (1_000_000_000_000, "T"),
        (1_000_000_000, "G"),
        (1_000_000, "M"),
        (1_000, "K"),
    ];
    for (scale, suffix) in UNITS {
        if n >= scale {
            return format!("{:.1}{}", n as f64 / scale as f64, suffix);
        }
    }
    n.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_path_aliases_are_rejected_before_search() {
        use std::path::Path;
        assert!(same_output_path(
            Path::new("metrics-test.json"),
            Path::new("./metrics-test.json")
        )
        .unwrap());
        assert!(!same_output_path(
            Path::new("metrics-test.json"),
            Path::new("other-metrics-test.json")
        )
        .unwrap());
        #[cfg(windows)]
        assert!(same_output_path(
            Path::new("metrics-test.json"),
            Path::new("METRICS-TEST.JSON")
        )
        .unwrap());
    }
    #[test]
    fn cli_validation_and_normalization() {
        let args = Args::try_parse_from(["words-breaker", "--resume"]).unwrap_err();
        assert_eq!(args.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        for (flag, value) in [
            ("--max-candidates", "0"),
            ("--batch-size", "0"),
            ("--block-size", "65"),
        ] {
            assert!(Args::try_parse_from(["words-breaker", flag, value]).is_err());
        }
        let wordlist = Language::Spanish.word_list();
        let w = wordlist
            .iter()
            .find(|w| w.nfc().collect::<String>() != **w)
            .unwrap();
        let composed = w.nfc().collect::<String>();
        let pattern = [composed.as_str(); 12].join(" ");
        let args = Args::try_parse_from([
            "words-breaker",
            "--language",
            "spanish",
            "--pattern",
            &pattern,
        ])
        .unwrap();
        let (slots, _, _) = build_template(&args, wordlist).unwrap();
        assert!(slots.iter().all(|s| *s == slots[0]));
    }
    #[test]
    fn cpu_finds_known_vector_and_respects_limit() {
        let language = Language::English;
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let m = Mnemonic::parse_in_normalized(language, phrase).unwrap();
        let slots: [Slot; 12] = m
            .word_indices()
            .map(|i| Slot::Fixed(i as u16))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let target = eth::parse_address("0x9858EfFD232B4033E47d90003D41EC34EcaEda94").unwrap();
        let args = Args::try_parse_from(["words-breaker", "--cpu", "--threads", "2"]).unwrap();
        let mut progress =
            checkpoint::Progress::new(slots, &[], &[], "english", target, None, false).unwrap();
        assert!(run_cpu_search(
            &args,
            &mut progress,
            &slots,
            &[],
            language.word_list(),
            language,
            &target
        )
        .unwrap());
        let mut open = slots;
        open[11] = Slot::Hole;
        let mut progress =
            checkpoint::Progress::new(open, &[], &[0, 1, 2, 3], "english", [0; 20], None, false)
                .unwrap();
        let args = Args::try_parse_from([
            "words-breaker",
            "--cpu",
            "--threads",
            "2",
            "--max-candidates",
            "2",
        ])
        .unwrap();
        assert!(!run_cpu_search(
            &args,
            &mut progress,
            &open,
            &[],
            language.word_list(),
            language,
            &[0; 20]
        )
        .unwrap());
        assert_eq!(progress.checked, 2);
        assert!(progress.candidates.next().is_some());
    }
    #[test]
    fn batch_cli_validation_and_known_phrase() {
        let language = Language::English;
        let m = Mnemonic::from_entropy_in(language, &[0x5a; 16]).unwrap();
        let words: Vec<_> = m.words().collect();
        let post = format!(
            "{}@1 {}@2 {}@3 {}@4 {} {}",
            words[0], words[1], words[2], words[3], words[4], words[5]
        );
        let video = format!(
            "{} {} {}@9 {}@10 {}@11 {}@12",
            words[6], words[7], words[8], words[9], words[10], words[11]
        );
        let args = Args::try_parse_from([
            "words-breaker",
            "--post",
            &post,
            "--video",
            &video,
            "--cpu",
            "--threads",
            "2",
        ])
        .unwrap();
        let plan = build_plan(&args, language.word_list()).unwrap();
        let target = eth::address_from_seed(
            &Secp256k1::new(),
            &eth::ETH_PATH.parse().unwrap(),
            &m.to_seed_normalized(""),
        )
        .unwrap();
        let mut progress =
            checkpoint::Progress::from_plan(&plan, "english", target, false, None, false).unwrap();
        assert!(run_cpu_search(
            &args,
            &mut progress,
            plan.slots(),
            &plan.report_pool(),
            language.word_list(),
            language,
            &target
        )
        .unwrap());
        assert!(Args::try_parse_from(["words-breaker", "--post", &post]).is_err());
        for flag in ["--pattern", "--pool", "--fill"] {
            assert!(Args::try_parse_from([
                "words-breaker",
                "--post",
                &post,
                "--video",
                &video,
                flag,
                "abandon"
            ])
            .is_err());
        }
        for bad in [
            "abandon@0",
            "abandon@13",
            "abandon@x",
            "abandon@1 abandon@1",
            "abandon@9",
            "abandon",
            "abandon@1 abandon@2 abandon@3 abandon@4 abandon@5 abandon@6 abandon@7",
        ] {
            let args =
                Args::try_parse_from(["words-breaker", "--post", bad, "--video", &video]).unwrap();
            assert!(
                build_plan(&args, language.word_list()).is_err(),
                "accepted {bad}"
            );
        }
        // Both selectors normalize composed Unicode before looking up words.
        let language = Language::Spanish;
        let word = language
            .word_list()
            .iter()
            .find(|w| w.nfc().collect::<String>() != **w)
            .unwrap()
            .nfc()
            .collect::<String>();
        let spec = vec![word; 6].join(" ");
        let args = Args::try_parse_from([
            "words-breaker",
            "--post",
            &spec,
            "--video",
            &spec,
            "--language",
            "spanish",
        ])
        .unwrap();
        let plan = build_plan(&args, language.word_list()).unwrap();
        assert_eq!(plan.stream().count(), 1);
    }
    #[test]
    fn unchecked_cpu_derives_original_words_without_repairing_checksum() {
        let language = Language::English;
        let phrase = ["abandon"; 12].join(" ");
        let indices = [0; 12];
        assert!(candidate_mnemonic(&indices, language, false).is_none());
        let m = candidate_mnemonic(&indices, language, true).unwrap();
        assert_eq!(m.to_string(), phrase);
        let target = eth::address_from_seed(
            &Secp256k1::new(),
            &eth::ETH_PATH.parse().unwrap(),
            &m.to_seed_normalized(""),
        )
        .unwrap();
        let plan = Plan::Template {
            slots: [Slot::Fixed(0); 12],
            pool: vec![],
            fill: vec![],
        };
        for no_checksum in [false, true] {
            let mut args =
                Args::try_parse_from(["words-breaker", "--cpu", "--threads", "2"]).unwrap();
            args.no_checksum = no_checksum;
            let mut progress =
                checkpoint::Progress::from_plan(&plan, "english", target, no_checksum, None, false)
                    .unwrap();
            assert_eq!(
                run_cpu_search(
                    &args,
                    &mut progress,
                    plan.slots(),
                    &[],
                    language.word_list(),
                    language,
                    &target
                )
                .unwrap(),
                no_checksum
            );
        }
    }

    #[test]
    fn cpu_exclusion_counts_original_candidates_and_continues() {
        let language = Language::English;
        let args = Args::try_parse_from([
            "words-breaker",
            "--cpu",
            "--threads",
            "2",
            "--batch-size",
            "1",
            "--pattern",
            "dutch ? ? cattle fog lake also forest wood fiber fork parrot",
            "--pool",
            "update winter",
        ])
        .unwrap();
        let plan = build_plan(&args, language.word_list()).unwrap();
        let target = eth::parse_address("0x9c2f44efad0c1e852a09df9939e6daf061140caf").unwrap();
        let rule = history::Exclusions::ro1().unwrap();
        let expected = plan.stream().filter(|p| rule.contains(p)).count();
        assert_eq!(expected, 1);
        let mut progress = checkpoint::Progress::with_history(
            &plan,
            "english",
            target,
            false,
            None,
            false,
            Some(rule),
        )
        .unwrap();
        assert!(!run_cpu_search(
            &args,
            &mut progress,
            plan.slots(),
            &plan.report_pool(),
            language.word_list(),
            language,
            &target
        )
        .unwrap());
        assert_eq!((progress.checked, progress.excluded), (2, 1));
        assert!(progress.candidates.next().is_none());

        let phrase = Mnemonic::from_entropy_in(language, &[0; 16]).unwrap();
        let slots: [Slot; 12] = phrase
            .word_indices()
            .map(|i| Slot::Fixed(i as u16))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let witness_target =
            eth::parse_address("0x9858EfFD232B4033E47d90003D41EC34EcaEda94").unwrap();
        let mut witness =
            checkpoint::Progress::new(slots, &[], &[], "english", witness_target, None, false)
                .unwrap();
        // Internal synthetic witness: CLI correctly rejects exclusions for this
        // target, but the engine must still find candidates outside the domain.
        witness.exclusion = Some(history::Exclusions::ro1().unwrap());
        assert!(run_cpu_search(
            &args,
            &mut witness,
            &slots,
            &[],
            language.word_list(),
            language,
            &witness_target
        )
        .unwrap());
        assert!(Args::try_parse_from(["words-breaker", "--exclude-tested", "C1"]).is_err());
        assert!(Args::try_parse_from([
            "words-breaker",
            "--coverage-report",
            "--checkpoint",
            "unused.json"
        ])
        .is_err());
    }
}
