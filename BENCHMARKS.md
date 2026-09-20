# Performance validation

Changes replace recursive, allocating candidate generation with a distinct-word
state machine. CUDA now uses portable 64-bit carries and separate seed/address
launches. Crypto algorithms and BIP-39 parameters remain unchanged.

`cargo bench --no-default-features --bench candidates` compares the original
iterator (test-only) with the new iterator. Each sample generates one million
candidates; the result is the median throughput of five samples, in release mode.
It uses `black_box`, excludes iterator construction, and never derives wallet keys.
The order changes for fill searches; these are cost-per-emitted-candidate results,
not timings to find a particular target. New enumeration also removes duplicates.

Measured on the local Windows host with Rust 1.98, 2026-09-06:

| Shape | Old candidates/s | New candidates/s | Enumeration speedup |
|---|---:|---:|---:|
| 12 known distinct words | 15,483,016 | 32,152,893 | 2.08x |
| 10 known words, two fill slots | 7,085,595 | 46,310,447 | 6.54x |
| Fill only | 3,906,847 | 72,065,320 | 18.45x |

These gains apply to candidate generation. They are **not full CUDA search
speedups**: generation already overlaps GPU computation, and PBKDF2 may dominate.
Native CUDA validation and measurements are now available below.

Validation includes exhaustive comparison against an independent multiset oracle
for small domains (including repeated words, overlapping sets and empty sets),
serialized cursor restoration at every boundary in several complete small spaces,
CPU checksum comparison with BIP39 vectors across all supported languages, known
Ethereum address vectors and bounded CPU search. The CUDA selftest now includes
the actual search pipeline and has passed on the local GPU.

Checkpoint version 1 describes the new traversal, not legacy candidate indices.
Completed batches only are committed. Changing the target or search inputs is
rejected on resume; changing execution settings is allowed.

Additional allocation instrumentation over 100,000 emitted candidates confirmed
zero allocation calls in the new iterator after construction for the three
scenarios examined (12 known words, 10 known plus two fill slots, two fill-only
slots). The historical 10-known case made 200,210 allocation calls requesting
412,621,644 cumulative bytes. These figures are not peak resident memory.

All 12 CPU tests passed in debug and release; Clippy passed with warnings denied.
CLI checks passed for a public known mnemonic, bounded search, checkpoint resume,
and resuming an already exhausted space.

## Native Windows CUDA measurements

Hardware: RTX 3060 Laptop GPU, 6 GiB, compute capability 8.6; driver 595.79.
Build: CUDA 12.8.93, MSVC 14.42, Rust 1.98, release, `sm_86`, 2026-09-06.
The toolkit is local to `.cuda/`; the NVIDIA driver was not changed.

The baseline here is the **combined CUDA kernel after the Windows arithmetic
port**, already using the new candidate iterator. The current version separates
PBKDF2 from BIP32/address derivation, passing 64 bytes per checksum survivor in
a reusable device buffer. This isolates the incremental CUDA change; it does
not measure the iterator improvement again. Offline `ptxas --verbose` reported
183 registers/thread for the combined kernel, versus 128 for the separate seed
kernel and 183 for the address kernel, with zero register spills in each.
The actual driver JIT reports 128 registers for seeds and 201 for addresses;
the benchmark uses the driver's compiled PTX, not the offline cubin.

Each run checks exactly 4,194,304 candidates from the same prefix of the 12-word
permutation stream, using the zero address as a negative target and the public
words in `scripts/benchmark-gpu.ps1`. Rates include initialization, enumeration,
copies, checksum filtering, PBKDF2, BIP32 and comparison. JIT cache warm-up is
excluded; no checkpoint writes are enabled. Versions run back-to-back and their
order alternates across three repetitions; settings also reverse order.

| Block | Batch | Combined median candidates/s | Separate median candidates/s | Change |
|---:|---:|---:|---:|---:|
| 32 | 1,048,576 | 1,344,203 | 1,375,905 | +2.36% |
| **64** | **1,048,576** | **1,359,337** | **1,391,012** | **+2.33%** |
| 128 | 1,048,576 | 1,353,459 | 1,383,541 | +2.22% |
| 256 | 1,048,576 | 1,430,133 | 1,382,068 | -3.36% |
| 64 | 262,144 | 1,259,550 | 1,360,298 | +8.00% |
| 64 | 2,097,152 | 1,429,618 | 1,427,894 | -0.12% |

Defaults remain **block 64 / batch 1,048,576**. The notebook heated up during
continuous testing: observed GPU temperature reached 88 C and SM frequency fell
to about 900 MHz. Initial runs exceeded 2 million candidates/s, whereas sustained
runs were around 1.3–1.4 million. Three samples are not enough to claim a small
block-size difference is universal, and these measurements are not a guaranteed
sustained rate for other cooling/power conditions. Raw paired results are in
[gpu-windows-rtx3060.csv](benches/results/gpu-windows-rtx3060.csv); its quoted
`Seconds` column uses the host's decimal comma. Rates are integer candidates/s.

An additional experiment reused a 16-round SHA-512 body instead of unrolling
all 80 rounds. All correctness tests passed, but five alternating comparisons
against the separate-kernel version gave medians of 1,353,219 versus 1,377,438
candidates/s (**1.76% slower**). It was reverted; the shipped code retains the
80-round implementation. See [experiment data](benches/results/gpu-sha512-16-rounds.csv).

The native GPU selftest passes SHA-256/512, Keccak, HMAC, PBKDF2, field arithmetic
against an independent BigUint oracle (including carry boundaries), compressed
and uncompressed public keys, scalar addition and known Ethereum addresses. It
also passes the real search across all ten languages, dense survivors, every
supported block size, batch boundaries, negative targets, limits, completed
cursor restoration and producer cleanup after a callback error.
CLI integration also found the public `abandon ... about` reference mnemonic,
saved 5 candidates on GPU, resumed another 8 on GPU with a different batch size,
and completed the remaining 14 on CPU: exactly 27 unique candidates in total.

Reproduce current configuration comparisons:

```powershell
./scripts/setup-cuda.ps1
./scripts/build-windows.ps1 -Check -Selftest
./scripts/benchmark-gpu.ps1 -OutputCsv results.csv
# Optional comparison with a separately saved baseline executable:
./scripts/benchmark-gpu.ps1 -BaselineBinary ./baseline.exe -DefaultOnly -Repeats 5 -OutputCsv comparison.csv
```

The scripts use a writable local JIT cache so repeated runs do not recompile
PTX. NVIDIA documents `CUDA_CACHE_PATH` and the first-run cost in its
[JIT cache environment variables](https://docs.nvidia.com/cuda/cuda-programming-guide/05-appendices/environment-variables.html).

## Upstream update integration (855fe1e)

The `--post` / `--video` iterator was adapted to the local allocation-free
traversal and checkpoints. A test-only copy of the upstream iterator lives in
`benches/support/upstream_two_pools.rs`. For eight holes, four words from each
of two disjoint eight-word pools, median enumeration throughput over five
one-million-candidate samples was **12,570,521/s upstream versus 42,905,933/s
locally (3.41x)**. Iterator construction is excluded, order differs, and these
are enumeration rates, not full-search rates. Both enumerate the same set;
small domains are checked exhaustively against an independent assignment oracle.

The existing scenarios also remain faster than their historical allocating
baseline in this run: 32,553,355/s (12 known), 39,935,624/s (10 known + fill),
and 97,795,685/s (fill only). Clock/load differences mean these should not be
used as a before/after comparison against earlier measurements on this page.

Integration tests cover quotas, repeated/shared words, exact counts, overflow,
cursor restoration at every boundary, selector validation and Unicode, planted
CPU/GPU hits, unchecked phrases without checksum repair, old checkpoints and
new-mode checkpoint policy mismatches. CUDA tests exercise checksum enabled
and disabled in every supported language and the two-pool iterator itself.
