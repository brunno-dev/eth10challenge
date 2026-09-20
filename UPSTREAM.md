# Upstream integration — 2026-09-06

## Review and research update — 2026-09-19

The original engine is still at `855fe1e`; no newer engine changes were found.
Added the puzzle repository's video word lists from `740c58b` (2026-09-07),
with pinned provenance, source hash and attribution in [history/research](history/research/README.md).
The GUI and MCP expose the four candidate groups without turning them into filters.
The 2026-09-09 oracle change (`789119f`) already matches our `--no-checksum`
capability. The desktop now shows checksum scope and prepares a separate expanded
search; native/MCP validation disables RO1 for that policy. Historical RO1
evidence, rules and existing checkpoints remain unchanged.

## Original integration

Source: [lmajowka/eth10challenge, 855fe1e](https://github.com/lmajowka/eth10challenge/commit/855fe1e6a084d97b587df7910581e8c743079205),
authored 2026-09-06 12:33:45 +03:00 (`change usage pattern`). The earlier README
commit `1a6374d` was also reviewed. Local HEAD before integration was `8ea7e47`.
Changes were adapted in the working tree; local optimizations were preserved.

## Included behavior

- `--post` / `--video`: each origin supplies six words, including its `word@N`
  pins. Unpinned words are drawn without replacement, with subsets when a pool
  has surplus entries. Hole-to-origin assignments are searched automatically.
- `--no-checksum`: CPU and GPU derive the original phrase even when its BIP-39
  checksum is invalid. The default keeps the inexpensive checksum filter.
- Updated usage and challenge context, with upstream provenance for historical
  challenge results. No challenge sweep was started as part of this integration.

## Adaptations to the local version

- Both modes use the allocation-free iterator, separate CUDA seed/address
  kernels, native Windows carries, limits and completed-batch checkpoints.
- Two-pool enumeration emits each distinct phrase once, including repeated and
  shared words. For k copies of a word, at least max(0,k-video_capacity) copies
  must come from post, and conversely for video. Tracking these lower bounds
  enforces quotas without branching over equivalent origin assignments.
- Counting uses multiset dynamic programming with overflow checks. The upstream
  factorial formula is exact only for disjoint pools with distinct entries.
- The upstream README example contains six, not eight, unpinned post words.
  Its distinct search space is **3,024,000**, not 14,112,000.
- Version-1 template checkpoints remain compatible. New checkpoints additionally
  bind both pools, their quotas and checksum policy; resuming with a changed
  policy is rejected so previously skipped candidates cannot be lost.
  The new modes write version 2 so older binaries reject their unknown semantics.
- In `--no-checksum` mode, GPU batches are capped at 65,536 candidates to keep
  full-derivation launches comparable in size to normal filtered batches.
- CPU verification of GPU hits also honors `--no-checksum`; reconstructing from
  entropy in that mode would silently change the last word and be incorrect.

## Upstream research notes

The updated [upstream README](https://github.com/lmajowka/eth10challenge/blob/855fe1e6a084d97b587df7910581e8c743079205/README.md)
and [CLAUDE.md](https://github.com/lmajowka/eth10challenge/blob/855fe1e6a084d97b587df7910581e8c743079205/CLAUDE.md)
add historical no-match pools and describe a sweep started on the author's
machine on 2026-08-25. These are upstream reports, not runs or active processes
on this machine. Any quoted wallet balance or run duration is a historical
snapshot, not a fresh on-chain or performance measurement.

## Validation

Seventeen unit tests cover both old and new selectors, independent exhaustive
two-pool oracles, repeated/shared words, exact counts, overflow, every small
cursor boundary, legacy checkpoint import and new-policy mismatch rejection.
The CUDA selftest additionally checks the two-pool stream and checksum on/off
against CPU references in all ten languages. See `BENCHMARKS.md` for the
enumeration comparison with the upstream implementation.

Release tests and Clippy passed in the final build. The RTX 3060 selftest passed
all primitive and integrated search checks. CLI checks found an intentionally
invalid-checksum public test phrase on GPU, then searched a shared-pool space
of exactly seven phrases in stages of 2 (GPU), 3 (GPU), and 2 (CPU), preserving
the version-2 checkpoint. Resuming with a changed checksum policy was rejected.
