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

- `kSkipAnyCorruptedRecords` / `set_paranoid_checks(false)` / `set_verify_checksums(false)` / `ChecksumType::NoChecksum` — `ErrorKind::NotSupported` (G2).
- `delete_files_in_range` as SST unlink — we tombstone+compact instead (safer-divergent).
- Encrypt-at-rest **in the LSM**. Volume encryption only.

### RFC-0038 P1.1 (owner, still open)

Surface is locked to **exactly two** modes: kernel `FailClosed`, compat `PointInTime` + `RecoveryReport`. Skip-any does not exist. CORRUPTLOG 3rd event refuses open in every mode. Evacuate (B2) and changing the product default stay the owner's call — this runbook will not pick A vs B2 vs B1+D.

`ingest_external_file` **is** implemented (WAL+flush, not a Rocks SST hardlink). Compaction filters run on `DB::compact`.

A Rocks C++ directory is **not** openable as Pedra ([RFC-0186](../rfc/0186-rocks-to-pedra-v5-migrate.md)). Copy the visible snapshot:

```sh
cargo run -p pedradb-cli --features from-rocks -- migrate-from-rocks /path/to/rocks /path/to/pedra
```

## Encrypt at rest

**Permanent:** LUKS, FileVault, or cloud volume encryption. The engine **never** implements encrypt-at-rest inside the LSM (RFC-0062 P2.2 as documentation, not a cipher).

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
