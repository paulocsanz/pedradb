# Empty-prefix HashMap (RFC-0180 P0.64 attempt) — REFUSED

**When:** 2026-09-08  
**Peer:** Pedra async vs Rocks `sync=false`. Darwin DIAG.  
**Hypothesis:** unique-key Linux overwrite 0.557 is empty-prefix
`c/`/`ycsb/` in a BTree (`cf_prefix` has no NUL). HashMap insert +
`cached_point_ord` range would beat `log n` without the RFC-0154 P1.3
ycsb_e smash (that one fell back to a full tail merge).

**Result:** **reverted before commit.**

| shape | P0.62 HEAD | P0.64 HashMap | notes |
|---|---:|---:|---|
| overwrite_mc4 | 1.401 (221 k vs 158 k) | 1.303 (200 k vs 154 k) | host slow, Rocks ≲160 k |
| **ycsb_e** | (floor 15/15 **S**) | **0.165** (47 k vs 285 k) | scan tax; P1.3 again |

P1.3 (2026-08-30) was ycsb_e 15.5→4.01 still >1. Wiring `cached_point_ord`
into `iter_internal_iter_at` did **not** save the scan. Empty prefix is
point **and** range. HashMap unique-insert is never until ycsb_e stays
≥ pre on the same boot.

**Do not retry** empty-prefix HashMap as an overwrite P0.
