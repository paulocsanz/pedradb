# RFC-0046 P2.8 — remote read cache A/B (2026-08-22)

Example: `crates/pedradb-core/examples/rfc0046_p28_remote_cache_ab.rs`
(stdout below). Workload: 17 rounds rewriting k00000..k08191 (64 B
values), window 2 s, local cap 2 MiB, remote mirror on a StdEnv temp
dir; the ROUNDS-3 round's segment is dropped by the final round's cap
while still listed by the final pre-drop manifest — its k00000 version
(decisive at the detached read point, seq 122 880) lives only in the
remote. Bloom-positive in every covering segment: no prune can help,
which is exactly the case the cache targets. 3 interleaved phases of
40 reads each; cache leg warmed once per phase (64 MiB budget), refetch
leg at budget 0. Both legs answered the kept round's value in every
phase (correctness cross-check).

## Result (best of 3 phases)

| leg | µs/read |
|-----|---------|
| cached | 5 039.5 |
| refetch (pre-P2.8) | 9 807.1 |
| **ratio** | **1.9×** |

48 remote objects / 24 593 387 B; the cached entry is 1 segment /
745 472 B (the kept round's).

## Why only 1.9× end-to-end — and why the cache still matters

Both legs pay the same ~5 ms floor: the LOCAL may-affect segments
(bloom-positive — they hold k00000's later, above-snap versions) are
read and CRC-walked on every read regardless. What the cache removes
entirely is the remote object fetch + walk — the ~4.8 ms/read delta
here. Against a real object store that delta is the per-read network
round trip + egress, which was the pre-P2.8 cost of EVERY
below-watermark read of a remote-only key. The local-walk floor is the
known P2.6 leftover (overlapping key sets; a follow-up sparse key
index or a binary search over the key-ordered records would attack
it).

## Limitations

- Dirty box (1-min load ~12 during the measurement — the P0.4 v1
  arbiter's long window coexisted); the ratio is the claim, absolute
  µs are not.
- Remote is StdEnv on local disk (page-cached): the refetch leg's
  fetch cost is a warm file read, not a network hop — real S3 makes
  the cache's saving larger, not smaller.
- Single read point/segment cached (`cache_entries=1`): the workload
  reads one hot key; the LRU/eviction paths are covered by unit tests,
  not by this A/B.
- The second execution took ~17 min vs ~66 s for the first at the
  same constants: `compact_horizon` slows as the archive accumulates
  re-archival duplicates (the known re-archival wart — each GC pass
  re-archives versions still in the LSM). Untouched by P2.8; worth its
  own slice if it matters in practice.
