# Runbook — single-node Pedra + Montanha lab (RFC-0050 P0.6)

**Not a fleet runbook.** Local embed and lab TCP only. Encrypt-at-rest is the host volume.

## Embed (one process, one directory)

### Backup / PITR (`pedradb-ops`)

1. `BackupEngine::create_base_backup` — flush + checkpoint into `base-NNNNNN`.
2. After more durable writes (while they still live in WAL), `ship_wal` archives complete records.
3. `restore_pitr` copies the base, filters archived records to `sequence <= target`, writes `CURRENT.log`, open recovers.

Call `ship_wal` **before** flushes that would drop unarchived WAL, or take a new base after heavy flush. This is **not** multi-region cluster backup.

### Durability fence

A failed required WAL `fdatasync` (or explicit flush/compact I/O) **fences** the writer (`ErrorKind::Fenced` / `CoreError::DurabilityFenced`).

- `FenceClass::Transient` (ENOSPC, EINTR): compat `auto_resume_transient` (default on) may reopen; kernel stays fail-closed until `recover_from_fence` / reopen.
- `Persistent` / `Unknown`: manual `DB::resume()` (compat) or drop+reopen. The report has `uncertain_from..=uncertain_through` and `lost_writes` — do not guess.

### WAL corruption

| Mode | Who | Mid-WAL CRC |
|------|-----|-------------|
| `WalRecovery::FailClosed` | kernel default | open `Err` |
| `WalRecovery::PointInTime` | compat default | serve prefix + `last_recovery_report` (never silent) |

`CORRUPTLOG` journals fail-stop events; the 3rd event refuses open **in every mode**. Skip-any does not exist (G2). Torn tail is recovered as a clean prefix (not journaled).

### What we will not do

- `ingest_external_file` — not implemented (`NotSupported`).
- `delete_files_in_range` — **unsafe to fake** (drop SSTs without tombstones). Use `delete_range` + compact.
- Compaction filters — Pedra GC is operator/explicit (`auto_reclaim` / `compact_reclaim`).

## Encrypt at rest

Ops-owned: LUKS, FileVault, cloud volume encryption. The engine does not implement encrypt-at-rest.

## Montanha TCP (lab)

```text
# Cleartext lab (default)
montanha-tcp node --id 1 --data /data --bind 127.0.0.1:9701 --peer 1=127.0.0.1:9701 ...

# TLS 1.3 + mTLS (build with --features tls)
montanha-tcp node --id 1 --data /data --bind 127.0.0.1:9701 \
  --tls-cert node.pem --tls-key node-key.pem --tls-ca ca.pem --require-tls \
  --peer 1=127.0.0.1:9701 ...
```

- `--require-tls` refuses to bind without cert/key/CA (GA profile). Cleartext remains the lab default when flags are omitted.
- Health HTTP (`GET /ready /leader /follower /status`) defaults to **127.0.0.1** (`bind_port+79`). Bind `0.0.0.0` only with explicit `--health`.
- Key rotation: replace PEM files and restart the process (no in-process rotation yet).

Without TLS on the wire this is **lab**, not GA.
