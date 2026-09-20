# GPU stage measurements and resident searches

Measured on 2026-09-20 with the local RTX 3060 Laptop GPU, CUDA 12.8,
release builds targeting `sm_86`. The new measurements separate PBKDF2 seed
generation from BIP32/address derivation. Two kernel experiments were rejected;
the production cryptographic kernel remains unchanged.

## What the timings measure

`gpu_seed_seconds` and `gpu_address_seconds` use CUDA events on the existing
stream, queried after the existing synchronization. They cover completed
negative batches and are `null` on CPU. Measuring the boundary introduces no
additional host synchronization between the two kernels.

These device intervals are subsets of the host-side `derive_seconds`. Full
process wall time also includes initialization, history loading, enumeration,
transfers, checkpoint/record writes and process shutdown. Producer work can
overlap GPU work; summing every reported stage does not give total time.

The baseline spent **about 89% of the two measured kernels' combined time on
seed generation**, with about 11% on address derivation. This percentage is not
the share of total process wall time. Nsight Compute was not installed and was
not used: diagnostics came from CUDA events, driver JIT register attributes and
an offline `ptxas --verbose` baseline report.

## Kernel experiments

Each experiment used five alternating before/after pairs for each of the
`coins` and `combined` history workloads: 5,000,000 original candidates per
invocation, fixed batch 1,048,576 and block 64. Both variants had the same event
instrumentation, inputs and frozen history. That is 20 invocations per
experiment, 40 in total. The full checkpoints, negative-record digests and
raw/excluded/retained/checksum counts agreed in all 20 pairs.

Medians in seconds; each cell shows baseline → experiment:

| Experiment | Workload | Process wall | Seed kernel | Address kernel |
| --- | --- | ---: | ---: | ---: |
| `const` K512 table | coins | 2.102785 → 1.901753 | 1.105828 → 1.106988 | 0.141502 → 0.142360 |
| `const` K512 table | combined | 1.934053 → 1.927628 | 1.166445 → 1.172755 | 0.148107 → 0.147065 |
| Funnel-shift `rotr64` | coins | 1.881439 → 1.954592 | 1.085789 → 1.099362 | 0.140161 → 0.129570 |
| Funnel-shift `rotr64` | combined | 1.929746 → 1.982808 | 1.157802 → 1.161642 | 0.144160 → 0.138561 |

The `const` table did not reduce seed time or register usage: the driver still
reported 128 registers/thread for seeds and 201 for addresses. Although the
coins process-wall median improved in this sample, device timings did not, and
combined wall time changed by only −0.3%. This was insufficient evidence of a
kernel improvement across workloads, so the change was rejected.

The funnel-shift version reduced address registers from 201 to 167, retaining
128 for seeds. Address time fell by 7.6% for coins and 3.9% for combined, but
seed time rose by 1.3% and 0.3%; process wall time rose by **3.9% and 2.7%**.
Fewer registers and a faster secondary stage did not improve the complete
search, so this experiment was also rejected. Neither change is shipped.

All three binaries passed the GPU selftest, including SHA/HMAC/PBKDF2,
independent arithmetic references, a known Ethereum address, the actual search
pipeline in all supported languages, limits, history, cursor restoration and
cooperative pause. Correctness passing was necessary, but not sufficient to
accept a performance change. The three `__noinline__` guards and full SHA-512
80-round unroll remain intact.

The per-invocation measurements are preserved in
[gpu-kernel-experiments.csv](gpu-kernel-experiments.csv). Raw local logs,
checkpoints and records are under
`output/import-check/gpu-resident-20260920/{const-comparison,funnel-comparison}`.
The paired workload is implemented by `scripts/benchmark-history.ps1` with
`-Trials 5 -MaxCandidates 5000000 -BatchSize 1048576`. These short laptop runs
do not establish sustained rates across different temperatures or power limits.

## Resident queue execution

The file queue now uses one `--worker` process for sequential lines and
automatic resumptions. It retains the CUDA context, loaded module and fixed-base
curve table; per-search inputs, buffers, flags, filters and progress remain
separate. Ordinary CLI/manual searches keep their existing execution model.

The local JSONL protocol emits versioned `ready`, ID-tagged `log` and matching
`done` events. Callers keep stdin open until the final `done`; EOF requests a
cooperative stop. A failed or uncertain job is not automatically retried, and
GPU search errors invalidate the session. Pausing, cancelling or finishing the
queue releases the process. See [the protocol](../docs/RESIDENT-WORKER.md) for
the exact lifecycle and recovery rules.

The process-level benchmark used one binary with the original production kernel
for fresh-process and resident execution, three
synthetic pools with nine distinct open words and three fixed positions, and
the zero address. Each row exhausts 9! = 362,880 candidates through four jobs
limited to 100,000 candidates each: 1,088,640 candidates and 12 jobs per variant.
There were three alternating pairs, both with a warmed JIT cache; the separate
warm-up was excluded from timing. No history exclusions were used, and all
files stayed in a new isolated directory outside application history.

| Measurement | Fresh process per job | Resident process |
| --- | ---: | ---: |
| Median wall time, 12 jobs over three rows | 4.481129 s | 0.967944 s |
| CUDA initializations per variant | 12 | 1, followed by 11 reuses |

The ratio of median wall times is **4.63×**, or **78.4% less elapsed time**.
Every variant processed the same 1,088,640 candidates and 68,284 checksum
survivors. All three pairs passed complete-checkpoint, negative-digest and
per-job count comparisons; resident logs confirmed the single initialization
and 11 subsequent reuses.

The measured executable's SHA-256 was
`6b0a059e8af84c65748cdbee515a1fcf60ec0c574ee1e1e55bdfbb015937fb02`.
The three pairs are preserved in [resident-results.csv](resident-results.csv);
raw per-job timings and validation artifacts are under
`output/import-check/gpu-resident-20260920/resident-comparison`.

```powershell
node scripts/benchmark-resident.mjs --binary ./target/release/words-breaker.exe --output ./output/resident-comparison-new --trials 3
```

Its `results.json` records total and per-job wall times. Verification requires
CUDA, one initialization plus explicit context reuse for the resident jobs,
identical complete checkpoints and negative digests, and equal per-job candidate
and checksum counts. Process startup/shutdown is included in total wall time;
post-run comparison is excluded. The measurement isolates the engine's repeated
small jobs and does not include Tauri history preparation or queue polling.
It does not establish a 4.63× gain for one long search or a faster cryptographic
calculation: long runs already amortize process and CUDA startup costs.

## Integration validation

The current engine passed the full physical GPU selftest and both explicit
CUDA worker tests: one context across changes of target, language and checksum
policy, and owner disconnection after confirmed GPU batches. The ordinary CPU
and CUDA-build test suites and Clippy checks also passed.

The real Tauri queue passed six end-to-end cases on each backend through its
authenticated local bridge: negative searches with reuse, already-covered
inputs, an independently derived known hit, pause/resume/cancel, injected
worker failure with explicit recovery, and close/reopen without automatic
restart. Checkpoints and negative evidence were checked throughout. A pause
that races with a completed round can legitimately leave that run `limited`;
the queue must still remain paused with a consistent saved checkpoint.

These tests used disposable app-data directories, preserving the real history.
The integration-only RPC methods are compiled out of production builds.
The Tauri suite passed 30 tests and the MCP suite passed 16 tests. The harness
and its isolated-build instructions are in `scripts/test-resident-queue.mjs`.
