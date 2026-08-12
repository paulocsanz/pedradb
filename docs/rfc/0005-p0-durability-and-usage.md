# RFC-0005: P0 durability contract + usage docs (P0.5 / P0.6)

**Status:** done  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)

---

## P0.5 — Durability

- Documented on `db` module rustdoc.  
- Default `OpenOptions.sync = true` → `sync_all` after each put/delete/commit.  
- Tests:
  - `crash_after_sync_put_reopen_recovers` (`mem::forget` after put)
  - `crash_after_multi_key_commit_reopen_recovers_both`
  - `uncommitted_tx_not_on_disk_after_crash`

## P0.6 — Usage

- [`docs/usage.md`](../usage.md) — open/TX, durability table, secondary-index sketch.  
- CLI: `pedra demo [path]` multi-key TX + reopen check.

## P0 complete

Justify-use vertical is shippable: multi-key TX + crash recovery + docs.  
Next: P1 SST (real store beyond MemTable+WAL).
