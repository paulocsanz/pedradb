# Montanha FDB-layer substitution — Phases 1–3 (+A–E continuum)

**Status:** done (thin real proofs)  
**Updated:** 2026-08-14  

Not FDB field peer. Not full bindingtester. Not production Apple Record Layer.  
Not drop-in etcd/Scylla/ClickHouse/NATS.

| Phase | Proof | Entry point | Tests |
|-------|--------|-------------|--------|
| **1** FDB-shaped client | create_tx / get / set / clear / commit + Conflict / too-old / limit + SI | `pedradb_store::run_phase1_bindingtester_subset` | `phase1_bindingtester_subset_harness` |
| **1b** deeper subset | multi-key, `get_range` + overlay, range OCC, clear→set, **`clear_range`** | same harness (12 steps) | same |
| **2** etcd multiproc freeze | create/CAS/get/watch multi-process | `EtcdNeedFace` + smoke `etcd-need` | `multi_process_etcd_need_face_freeze` |
| **2b / A** etcd TCP wire | create exclusive + CAS + majority DcsGet over **real TCP** | `client_dcs_*` + `WireMsg::Dcs*` | `tcp_etcd_need_create_cas_get` |
| **3** Record seed | row + secondary index in one TX; concurrent Conflict | `RecordTable` | `phase3_record_table_*` |
| **3b / C** Record +1 | unique secondary index; two indexes in one TX | `upsert_unique` / `upsert_two_indexes` | `phase3_record_unique_and_multi_index` |
| **D** DST residuals | F47 abort fence mid-2PC; F48 AE durable; F49 si_gen Queued | store recovery / AE / si_gen | `fail_after_mid_2pc_*`, related |
| **E** platform need faces | Scylla-CP + OLAP RO + stream subject + etcd-need on one SoR | `cp_put` / `olap_*` / `stream_*` / `EtcdNeedFace` | `platform_need_faces_scylla_olap_stream` |
| **Bench** FDB-shaped limits | put/get/face/TX/range/clear/hot-key/record/cross-range + **batch** | `montanha-fdb-bench` | `findings/fdb-bench-*/fdb_shaped_bench.json` |
| **Perf P0** | `put_many` + lab open + batch benches | [RFC-0025](rfc/0025-montanha-perf-parity-vs-peers.md) | `put_many_*` tests |

## Honesty residual

- Phase 1/1b is a **subset** of FDB client semantics, not 100% bindingtester (no full op list, no directory layer, no fdb watches).  
- Phase 2/2b freezes dual SoR for **locks/config**, not full etcd gRPC wire.  
- Phase 3/3b is a **seed** (encoding + multi-key TX), not Java Record Layer / SQL planner.  
- Platform need faces replace the **job**, not the logo (no CQL / CH SQL / JetStream protocol).

## Related

- [montanha-fdb-recipes.md](montanha-fdb-recipes.md) — design recipes  
- [RFC-0023](rfc/0023-fdb-functional-tx-parity-and-compat-face.md) — TX physics  
