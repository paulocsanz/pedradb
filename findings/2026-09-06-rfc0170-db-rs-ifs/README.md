# RFC-0170 P2.4 — `if` de destino de dados ainda em `db.rs`

**Date:** 2026-09-06
**Constraint:** `glue.db_rs_extracted` remains `false`. Each slice extracts **one** decision to a named kernel.

## Extracted this slice

`Db::write_admission_idle` (stall knobs off ⇒ skip CF-family collection) now
calls `write_admission_kernel::write_admission_idle`. Production `db.rs` still
owns the `Option` knobs; the kernel sees three bools.

- kernel: `crates/pedradb-core/src/write_admission_kernel.rs`
- twin: `crates/pedradb-core/verus/write_admission.rs` (6 verified / 0 errors)
- plant: `write_admission_idle_on_live_stall_is_not_ok`
- Aeneas: `SOURCE.write_admission` + `WriteAdmission.lean` (no hole)

`Db::ensure_write_admitted_for` hard stall (mem then L0, after optional drain)
calls `write_admission_kernel::write_admit`. Drain/flush stays glue.

- entry: `write_admit` / as-is always `Ok`
- plant: `write_admit_on_live_mem_over_is_not_ok`
- Lean: `write_admit_mem_over_stalls` / `write_admit_as_is_dente`

## Remaining candidates (next slices, one each)

`db.rs` non-test is ~13k LOC / ~750 `if`. Already wired to kernels:
`flush_kernel`, `compact_kernel`, `group_commit_kernel`, `probe_order_kernel`,
`cf_kernel`, `vlog_gc_kernel`, `reopen_kernel`, `changelog_kernel`,
`write_admission_kernel`.

`Db::maybe_auto_flush` now asks `flush_kernel::auto_flush_due` (armed byte
limit reached). SST write / park stays glue. AS-IS never fires.

Next data-fate `if`s that still live in glue (not a catalog `entry`):

| Fn | Decision | Why it is data-fate |
|----|----------|---------------------|
| `maybe_auto_compact` | L0 count / SST bytes fire compact | whether versions disappear |
| `compact_family_key` | empty `physical_cfs` ⇒ `""` | compact grouping |
| `commit_ops_with` | `do_sync` fence after append | ack vs durability |
| `sync_dir_if_required` | `self.sync` ⇒ dir fsync | directory entry durability |
| `wal_sync_group` | group-commit sync error fence | same class as RFC-0015 H1 |

Do **not** extract `lookup_body` / `scan_at_raw` as one kernel — those are
glue over `probe_order_kernel` + merge. Split only a named predicate.

## Not this RFC

Extracting `db.rs` itself. Handler LOC staying ≫ kernel LOC is the product.
