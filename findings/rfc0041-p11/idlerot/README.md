# RFC-0041 — idle WAL rotate + uncompressed L0 (rejected for apply)

2026-08-17. `c30f483`. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3.

Idle-only WAL rotate is **kept** (drain with empty active mem was
fsyncing the new SST mid-apply). Uncompressed L0 (SST v3, skip lz4)
**regressed apply**: apply_mc4 0.74 → **0.46**, 1c apply p50 265 →
400–480 µs. Larger L0 write contended with the apply CPU. Reverted
to streamed v4 lz4 for L0; idle rotate + compact-every-L0-when-idle stay.

**3/16 ≥ 2.0** (C 2.99, E 2.35, MVCC 2.69). FLOOR not enabled.
