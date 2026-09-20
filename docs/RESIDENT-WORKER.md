# Resident search engine protocol

`words-breaker --worker` keeps one search process available for sequential jobs.
The Tauri file queue uses this mode between its lines and automatic resumptions;
ordinary CLI searches retain their existing behavior. The worker is a local
child-process protocol, not an HTTP or MCP server.

## Lifetime and resource reuse

The worker emits `ready` before any CUDA initialization. CUDA is initialized
lazily when a job needs it, and its context, loaded module and fixed-base
secp256k1 table remain in the session. They are created, used and destroyed on
the same OS thread. A separate input thread only reads requests and detects
caller disconnection; it never owns the GPU context.

The first GPU job logs `CUDA context initialized`; subsequent GPU jobs log
`Reusing initialized CUDA context`. Forced CPU jobs and searches already proven
covered do not initialize CUDA. If CUDA initialization is unavailable, the
session reports the reason and uses CPU; it does not retry initialization for
every following job. CPU jobs create a local Rayon pool, so consecutive jobs
may request different thread counts without rebuilding a global pool.

Only the GPU infrastructure is cached. Candidate buffers, wordlist data,
candidate plans, pruning structures, metrics and history selection are rebuilt
for each search. Compatible history files are read again, including negatives
confirmed by previous queue jobs. Context reuse therefore does not reuse stale
filters or a previous job's target, language, checksum policy or candidate
cursor. A resumed checkpoint still validates its original inputs and exclusion
fingerprint; `--additional-record-dir` supplies newer compatible negatives under
the existing resume rules.

The desktop releases the resident process when the queue completes, pauses,
is cancelled, finds a match or encounters an error. Reopening a queue starts a
new process and resumes only from its saved state.

## Transport and request ordering

Start the executable with **only** `--worker`. Keep stdin and stdout open and use
UTF-8 JSON objects, one per newline. Read the initial response before sending a
request:

```json
{"event":"ready","version":1}
```

Send one request at a time and wait for its `done` event before sending another.
Pipelining is unsupported; exceeding the bounded pending-input capacity cancels
the session instead of allowing input buffering to hide a disconnection.

```json
{"id":"line-1-round-1","args":["0x0000000000000000000000000000000000000000","--pattern","abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about","--cpu","--threads","1","--metrics"]}
```

`args` contains the same individual argument strings passed to the CLI, without
an executable name or shell quoting. A multiword pattern is one array element.
Use absolute paths for checkpoints, metrics, negative records and stop markers.
Each request must contain only `id` and `args`. Choose a distinct ID for each job.

Input bounds are:

| Item | Limit |
| --- | --- |
| JSONL frame | 1 MiB, including the line terminator |
| Request ID | 1–128 UTF-8 bytes, with no control characters |
| Argument count | At most 4,096 strings |
| Individual argument | At most 64 KiB of UTF-8, with no NUL |

The worker accepts search arguments only. Nested `--worker`, `--selftest`,
`--list-history`, `--preflight-history`, `--coverage-report`,
`--export-checkpoint-record`, help and version commands are rejected. Run these
commands in an ordinary process when needed. Normal CLI validation also applies
to every search request.

## Events and outcomes

All stdout output is JSONL. Existing stdout/stderr search diagnostics become
ordered `log` events, with multiline messages split into separate events:

```json
{"event":"log","id":"line-1-round-1","stderr":false,"line":"Using CPU with 1 threads"}
{"event":"done","id":"line-1-round-1","success":true,"error":null}
```

`success: true` means the search invocation returned normally. It does **not**
mean a wallet was found or the search was exhausted. The normal outcome logs,
checkpoint, metrics and confirmed-negative record distinguish a match, a limit,
a pause and full exhaustion. A match continues to require independent CPU
verification of the GPU result and is not published as a negative record.

Request errors are reported with the same valid ID:

```json
{"event":"done","id":"line-1-round-1","success":false,"error":"Missing target address"}
```

If a malformed, oversized or invalid-ID request cannot be correlated safely,
`done` contains `id: null`. The desktop treats unexpected IDs, malformed output,
unsupported versions, `done` failures and premature process loss as errors; it
stops the queue and discards that worker. It never attributes another job's logs
to the current search.

A validation error need not poison a standalone worker. An error during GPU
search does poison the session: the worker emits failed `done`, discards its GPU
and exits without attempting another job. A caught search panic is also fatal.
Process status alone is insufficient to establish a successful job: the caller
must receive a valid, matching `done` event and inspect the search outcome.

There is **no automatic retry** after a failed or uncertain invocation. Recovery
requires an explicit resume from the saved checkpoint. A failure or a missing
`done` does not establish new negative evidence.

## Pause, EOF and confirmed progress

Keep stdin open until the last job's `done`. **EOF cancels an active search**; it
does not mean “finish all queued requests.” The input reader marks the owner as
closed and creates the active job's `--stop-file`. When the caller did not supply
one, the worker supplies a private temporary marker and removes it afterward.
Existing caller-owned markers are never truncated or removed by the worker.

An active search observes the marker at its normal batch boundary and follows
the same pause/checkpoint path as a desktop pause. Only completed work advances
the checkpoint or contributes to a confirmed-negative prefix; producer
lookahead and unfinished batches do not. Candidate limits and counters retain
their existing meaning over the original search domain, including candidates
excluded by history or skipped by a coverage proof. Reusing the process does
not change those counts or turn an interrupted domain into complete coverage.

EOF while idle closes the worker. EOF before a received job starts prevents
that job from running. Once the owner is closed, the session cannot accept a
successor job. If the stop marker cannot be created, the worker exits rather
than leaving an orphaned search running; only checkpoints already saved can be
used for recovery. A caller should also treat broken protocol I/O as process
loss and must not automatically replay the request.

## Validation

`tests/worker_protocol.rs` drives the actual executable through stdin/stdout.
It covers correlated logs and outcomes, a known public mnemonic match followed
by a negative job with a different CPU thread count, recovery from malformed
input, and owner EOF during an active search with either private or caller-owned
stop markers. Unit tests in `src/worker.rs` cover argument/ID limits, rejected
control commands and bounded draining of oversized frames.

Two ignored integration tests require a physical CUDA device. After warming
the driver JIT cache, run them explicitly:

```powershell
cargo test --release --locked --test worker_protocol cuda_worker_ -- --ignored --test-threads=1
```

On the configured Windows toolkit, `./scripts/build-windows.ps1 -Check -Selftest`
also runs these physical-GPU tests after the ordinary suites and GPU selftest.
The build script restores the CUDA executable after CPU integration tests,
which otherwise replace the shared release binary with their CPU-only build.

They verify one CUDA initialization across English hit/miss, invalid-checksum
rejection and derivation, and Japanese jobs; they also check the separated GPU
timings. The EOF test waits for confirmed GPU batches before disconnecting the
caller and checks the saved partial-negative prefix against checkpoint and
metrics counts. These tests reject automatic CPU fallback.
