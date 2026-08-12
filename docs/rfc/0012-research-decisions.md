# RFC-0012 companion: measured research decisions (P2.1–P2.2)

**Status:** done  
**Updated:** 2026-08-11  

## Decision

| Item | Decision | Evidence gate |
|------|----------|---------------|
| Bloom filters | **Do not ship** in this horizon | Revisit only if range/point get profiles show SST full-scan cost dominates |
| Value log (WiscKey) | **Do not ship** | Revisit if value size distribution in production embeds is large-value heavy |
| Lazy Leveling | **Do not ship** | Compact remains whole-merge + count policy |
| MemTable skiplist/arena | **Keep BTreeMap** | Revisit if `benches/baseline` write path is CPU-bound in MemTable |

## How to re-open

1. Run `cargo bench -p pedradb-core --bench baseline` and attach numbers.  
2. Open a **new** RFC (0013+) with the bench claim; do not re-open 0009/0012 silently.

## Delivered

This document **is** the P2.1/P2.2 delivery: an explicit, testable non-ship with a reopen path — not a deferred `todo`.
