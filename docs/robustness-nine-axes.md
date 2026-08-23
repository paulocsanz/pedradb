# Nine robustness axes (RFC-0050)

**Updated:** 2026-08-23
**RFC:** [0050](rfc/0050-nine-axis-robustness.md)

This is the operator-facing scoreboard. **Lab-mature, not field-peer.** None of the rows below is a GA, TiKV drop-in, or Rocks/Pebble/FDB field claim.

Verify:

```bash
cargo test -p pedradb-core wal_recovery_exactly_two_modes -- --nocapture
cargo test -p pedradb-sim enospc_mid_ -- --nocapture
cargo test -p pedradb-io-uring -- --nocapture          # Linux: live ring + CQE inject
cargo test -p rocksdb-compat ingest_external -- --nocapture
cargo test -p pedradb-store --features tls --test tcp_multihost tcp_tls -- --nocapture
```

| Axis | Shipped (this wave) | Residual / wall |
|------|---------------------|-----------------|
| 1 Campo | FailingEnv, DST, Darwin `fdatasync` class, io_uring soak | Years of multi-tenant, bad disks, firmware, real ENOSPC, hardware plug-pull. **Not programmable.** |
| 2 Disco Linux | `FailingEnv::wrap(IoUringEnv)` + CQE `res<0` → EIO/ENOSPC, not Ok. Soak job on ubuntu. `det_io_status.sh` residual if sibling missing | QEMU guest image; `dm-error` / page-cache block EIO; CONTRACT-OK is **not** default CI |
| 3 Disponibilidade | Named ENOSPC/EIO tests mid-flush / compact / MANIFEST; explicit flush/compact I/O **fences**; range-delete compact terminates with L0 stall | Rocks quarantine theater; fleet background-error; multi-TB multi-policy compact |
| 4 Escrita | `ConcurrentDb` group commit (compat already wired — gap #9 was stale); contract test: groups &lt; submits; apply still in the write lock | Not the Rocks concurrent memtable / multi-flush writer. RFC-0045 P2.1 is +15% expected, not 5× |
| 5 Cluster | Majority / dual-leader / I-MAJ **lab**; TLS 1.3 + mTLS behind `--features tls` | Not production multi-Raft; not FDB Simulation; not zero-downtime range HA. Cleartext remains lab default. **No GA without `montanha-secure` + fleet.** |
| 6 Ops | Local backup / PITR / WAL-ship (`pedradb-ops`); [runbook](runbooks/single-node-and-montanha-lab.md) | No multi-region cluster backup; no `ingest_external_file`; no compaction filters |
| 7 WAL mid-corrupt | Kernel `FailClosed`; compat `PointInTime` + `RecoveryReport`; CORRUPTLOG escalation; **exactly two modes** | RFC-0038 P1.1 (PIT vs fail-stop as **product default**) is the owner’s. Skip-any is forbidden (G2). No silent third mode |
| 8 Face Rocks/TiKV | rust-rocksdb 0.22 API: ingest/`SstFileWriter`, `delete_file_in_range` (tombstone+compact), WBWI, compaction filter, `create_cf`/`drop_cf`, merge, multi_get, properties | Prefix CFs (not physical files per CF). Titan knobs are no-ops. A TiKV **cluster** is still integration work, not a missing method. |
| 9 Disco / wire | Encrypt-at-rest = LUKS/volume (ops); TLS on MTCP when flags set; health HTTP default **127.0.0.1** | Engine has no encrypt-at-rest. TCP without `--tls-*` is cleartext lab |

Official Rocks win remains vs default `WriteOptions.sync=false` (AGENTS.md). Sync-peer ratios are not a win.
