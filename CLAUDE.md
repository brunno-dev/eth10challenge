# CLAUDE.md

GPU brute-forcer that recovers a 12-word BIP-39 mnemonic from a known Ethereum
address by enumerating word orderings. Built to attack the Guntis Vitolins
"10 ETH challenge" (published 2020-02-12, YouTube `w4mpiuBP_aY`).

## Build & run

```bash
cargo build --release
./target/release/words-breaker --selftest          # verify GPU primitives vs CPU
./target/release/words-breaker <ADDR> <12 words>   # loose-word mode
./target/release/words-breaker <ADDR> --pattern "dutch ? ? ? fog ? ? ? ? ? ? parrot" --pool "..."
```

`nvcc` is required for the default CUDA build; use `--no-default-features` for
a CPU-only build. Native Windows setup/build scripts are in `scripts/` and have
been validated on an RTX 3060 Laptop GPU. `CUDA_LIBRARY_PATH` is set in
`.cargo/config.toml` to `/usr/lib/cuda` (distro CUDA layout). `--cpu` forces the
rayon path. See README.md for the full flag table.

## Layout

| File | Role |
|---|---|
| `src/main.rs` | CLI, pattern/pool parsing, address checksum validation |
| `src/candidates.rs` | the single enumerator: 12 slots (pinned or open) + pool drawn without replacement + `--fill` set for leftovers |
| `src/gpu.rs` | CUDA host side: batching, producer thread, checksum pre-filter, `--selftest` |
| `src/cuda/kernels.cu` | Crypto primitives; `k_filter`, `k_candidate_seeds`, `k_seed_addresses` |
| `src/eth.rs` | CPU reference implementation (also the selftest oracle) |
| `src/worker.rs`, `src/output.rs` | Sequential JSONL worker and correlated diagnostics; CUDA session stays on one thread |

Derivation is fixed at `m/44'/60'/0'/0/0`, no BIP-39 passphrase.

## Upstream integration (2026-09-06)

Ported `855fe1e` from lmajowka/eth10challenge; see `UPSTREAM.md` for provenance.
`--post` and `--video` each supply exactly six words, counting `word@N` pins.
Bare words form each origin's pool; no fill fallback. `src/plan.rs` selects the
mode, and `stream_two_pools` preserves distinct enumeration across overlapping
or repeated words. Factorials count assignments and overcount such inputs.
The upstream README's sample has SIX unpinned post words; its count is 3,024,000.

`--no-checksum` skips the filter on both backends and preserves the exact words
when deriving/verifying a hit. Never rebuild unchecked phrases from entropy:
that repairs the final word. GPU batches in this mode cap at 65,536. Checkpoints
bind checksum policy and both origin pools; old template checkpoints still work.
The author's newly reported negative searches and sweep are linked in
`UPSTREAM.md`; do not treat that remote sweep as a local running process.

## Traps

- **Do not remove `__noinline__`** from `pbkdf2_hmac_sha512_64`,
  `seed_to_eth_address`, or `keccak256`. Inlined into the old combined kernel, `nvcc -O`
  miscompiles PBKDF2 and the seed comes out wrong — but the standalone kernels
  stayed correct while real searches silently found nothing. The current
  selftest also checks the complete search pipeline. Preserve the guards.
- **PBKDF2-HMAC-SHA512 dominates derivation** (2048 iterations, fixed by
  BIP-39). The historical estimate was 92–94%; current RTX 3060 Laptop CUDA
  events measure about 89% seed / 11% address across the two kernels, not total
  process wall time. See `history/GPU-RESIDENT.md`. Profile first; check
  `ptxas --verbose` after touching search kernels (historical RTX 3050 baseline:
  2720-byte frame, 128 registers, 0 spills).
- **WSL copy artifacts**: this tree came from Windows. `target/` build scripts
  lose their exec bit (`find target -name "build-script-build*" -not -name "*Zone.Identifier" -exec chmod +x {} \;`,
  same for `*.so`) — chmod, do not `cargo clean`. ~894 `*:Zone.Identifier`
  files litter the tree; ask before deleting them.
- The pool is drawn **without replacement**. When it fills all holes, a repeated
  word requires repeated pool entries; additional fill positions may repeat.
- Current Windows measurements and rejected experiments are in `BENCHMARKS.md`.
  The 16-round SHA-512 loop experiment was slower; retain the 80-round unroll.
- The resident worker only caches CUDA infrastructure. Every job reloads its
  inputs and compatible history. EOF requests a stop; a failed GPU search
  invalidates the session. Never automatically replay an uncertain job or
  turn a hit/interrupted batch into negative evidence. See `docs/RESIDENT-WORKER.md`.

## Cost model

Measured 2.42M candidates/s on an RTX 3050 (~151k full derivations/s after the
1-in-16 checksum filter). The full 12! space is ~197 s.

With `h` open slots, pool `p`, fill set `f`: `p!/(p-h)!` when `p >= h`, else
`h!/(h-p)! * f^(h-p)` is an assignment upper bound: repeated words and pool/fill
overlap reduce the distinct space. The CLI now counts it exactly with multiset
DP. Historical estimates below use that upper bound. With four pins (**8 open slots**) and free slots
drawn from the 218 `d*`/`f*` words:

| unknown slots | space | time |
|---|---|---|
| 0 | 40,320 | 0.02 s |
| 1 | 8.8M | 3.6 s |
| 2 | 9.6e8 | 6.6 min |
| 3 | 7.0e10 | 8.0 h |
| 4 | 3.8e12 | 18 days |

These historical estimates assume `fiber`@4. That pin is a hypothesis, not a
confirmed position: hint 4 identifies the word and allows any seed position.
Removing it increases the search space; do not use this table for three-pin runs.

If a candidate *list* of size C supplies the words, don't forget the `C(C, k)`
choose-factor — it dominates. With `fork` confirmed and U of the 7 remaining
slots unknown, the cost is `C(C, 7-U) * P(8, 1+(7-U)) * 218^U`; that first
factor is the one that is easy to drop and it is worth orders of magnitude.

## Challenge state

Target `0x9C2F44EFAd0c1E852a09dF9939e6DaF061140CaF`, confirmed on-chain to hold
8.612541554256944620 ETH.

### Positions used by the published RO1 model

| Position | Word |
|---|---|
| 1 | `dutch` |
| 5 | `fog` |
| 12 | `parrot` |

Positions 2–4 and 6–11 are open — **9 open slots**. These anchors define the
RO1 model, not evidence that all other hypotheses can be discarded. A negative
search does not prove a positional hint. `fiber@4` was previously mislabeled
as certain here; see the archived author reply in `history/ro1/author-posts.md`.

### Known words (position unknown)

`fork` and `fiber` are required members in RO1, both without fixed seed
positions. Together they leave 7 other unknown words among the 9 open slots.

### Extraction rule

Both confirmed words (`dutch`, `fiber`) appear as exact verbatim tokens in the
planted sentences, so the rule is: **take every token that is an exact BIP-39
word, grammatical connectors included.** BIP-39's 4-letter uniqueness means
stems resolve (`healthy`→`health`, `hunter`→`hunt`).

Blog text ("Round dutch cattle is living in the forest and eating wood. Only
because there is a lot of healthy fiber. Hunter like the rib roast dinner
fresh.") → 14 words:
`round dutch cattle forest wood only because there fiber like rib roast dinner fresh`

Video fragment ("Don't expect anything easy there will be dark fog on the
lake") → exactly 6, matching the 6 words the puzzle hides in the video:
`expect easy there will fog lake`

`parrot` appears in neither fragment, so the source text we have is incomplete.
The published research says part of the pool hides in the blog post's HTML
`article:tag` metadata rather than visible text.

### Historical negative reports (external, conditional on their inputs)

All reportedly against the target above, all no match. The following prose
is not sufficient for automatic exclusion; exact inputs and coverage matter:

- pool `fork fiber forest dinner goat seed key lake` with `fog`@5 (743M), and
  the same with `cloud`@5, with `dinner` but no `goat`, and with `dutch` added
  to the pool
- position 5 unpinned, `fog` or `cloud` floating over 10 slots (3.6M each)
- 13-word pool (+`round wood cattle roast fresh`) and 14-word (+`dutch`), with
  `fog`@5 and `cloud`@5 (259M / 726M each)
- the all-`d`/`f` theory `dutch fog parrot fork fiber forest dinner fresh donor
  favorite find decide`: pinned (363K), **all 12 words free in every ordering
  (479M)**, and dropping each unpinned word for any `d*`/`f*` replacement
  (9 × 79M)
- pins held, pool of `fork fiber` + candidates incl. `deliver`, `detail`,
  `digital`, `day`: pools of 10/11/12/13 → 3.6M / 20M / 80M / 259M

Do not infer that these reports close new pools, repeat policies or derivation
settings. Searches allowing `fiber` to float include its position-4 subset;
searches pinning it at 4 do not cover its other positions.

The 479M full-freedom run is the decisive one: **with positions fully free the
word set itself is wrong.** The per-word drop runs then show it is not a single
wrong word among the nine unpinned ones either. So ≥2 words are wrong, or one
of the three pins is — and given the pins are corroborated, the pool is the
likelier error.

### Untried levers, cheapest first

1. Unpin position 5 and let `fog`/`cloud` float over 10 slots: `P(14,10)`=3.6e9
   (~25 min), `P(15,10)`=1.1e10 (~75 min).
2. Get the *complete* video transcript and blog post source (HTML metadata
   included) and re-run the extraction rule. This is the highest-value move —
   the blocker is the word pool, not throughput.
