# Montanha FDB-layer substitution — Phases 1–3

**Status:** done (thin real proofs)  
**Updated:** 2026-08-14  

Not FDB field peer. Not full bindingtester. Not production Apple Record Layer.

| Phase | Proof | Entry point | Tests |
|-------|--------|-------------|--------|
| **1** FDB-shaped client | create_tx / get / set / clear / commit + Conflict / too-old / limit + SI | `pedradb_store::run_phase1_bindingtester_subset` | `phase1_bindingtester_subset_harness` |
| **2** etcd multiproc freeze | create/CAS/get/watch only on multi-process store | `EtcdNeedFace` + smoke `etcd-need` | `multi_process_etcd_need_face_freeze` |
| **3** Record seed | row + secondary index in one TX; concurrent Conflict | `montanha_fdb_recipes::RecordTable` | `phase3_record_table_*` |

## Honesty residual

- Phase 1 is a **subset** of FDB client semantics, not 100% bindingtester.  
- Phase 2 freezes dual SoR for **locks/config**, not full etcd gRPC.  
- Phase 3 is a **seed** (encoding + multi-key TX), not Java Record Layer / SQL planner.

## Related

- [montanha-fdb-recipes.md](montanha-fdb-recipes.md) — design recipes  
- [RFC-0023](rfc/0023-fdb-functional-tx-parity-and-compat-face.md) — TX physics  
