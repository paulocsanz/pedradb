# RFC-0171 P2.1 — trampoline restante em `db.rs`

**Date:** 2026-09-06
**Constraint:** `glue.db_rs_extracted=false`. Glue allowed = `Env` / syscall, like seL4 assembly.

## Put-Ok path (`put_with` → `apply_batch_with` → `commit_ops_with` → `alloc_seq`)

Data-fate predicates now live in `write_admission_kernel.rs` (single artifact):
`write_admission_idle`, `write_admit`, `wal_sync_required`, `seq_exhausted`,
`batch_is_empty`, `fence_on_sync_fail`.

What remains in those fns is trampoline: WAL `append_write_ops` / `sync_data`,
mem apply, fence I/O, telemetry, sequence counter mutation.

## Recover/reopen (`open_with_env_sourced`)

Kernel predicates: `torn_head_is_empty_log`, `torn_tail_needs_cut`,
`seq_after_feed`, `pit_resync_needs_rewrite`, plus existing `reopen_outcome`
/ `vlog_recover_action`.

Trampoline: `env.exists`, `metadata_len`, `open_append`, `set_len`, `sync_dir`,
`rename`, DirLock, MANIFEST/SST install.

## Freeze

`handler_loc` 102893 (strictly below freeze 103312 on HEAD `b9c1859a`).
`kernel_loc` 15715 (53 enrolled paths; write_admission 98 → 740).
