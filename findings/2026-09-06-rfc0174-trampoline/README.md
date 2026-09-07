# RFC-0174 P2.2 — get/scan trampoline inventory

`db.rs` remaining `if`s on the get path after P1.1. Data-fate predicates
live in `lookup_kernel.rs` / `merge::visible_at` / `probe_order_kernel`.
This file is Env / cache / bloom / packed-probe glue. Do not dump `db.rs`.

## get_at

| `if` | class |
|------|--------|
| `lookup_kernel::snap_is_empty` | kernel |
| `lookup_kernel::snap_below_watermark` | kernel |
| `published_seq` / `point_cache` | cache trampoline |
| `vlog::decode_vlog_ptr` | vlog trampoline |

Archive I/O (`get_at_from_archive`, `get_at_below_watermark_lsm`) lives in
`db/lookup_archive.rs` (submodule of `db`, not a catalog caller). History-tier
fetch, bloom sidecar, remote cache — not destination-of-data predicates.

## lookup / lookup_body

| `if` | class |
|------|--------|
| `lookup_kernel::prefer_newer_seq` | kernel |
| `lookup_kernel::mem_point_decides` | kernel |
| `merge::visible_at` / `range_deleted` | kernel (existing) |
| `probe_order_kernel::probe_order_covering` | kernel (RFC-0164) |
| `sst_only_settled` / bulk family | settled-mode routing |
| `if let Some(...)` | Option unpack |
| `key_may_match` / packed lo/hi / envelope | bloom + probe trampoline |
| TLS seek scratch | I/O trampoline |

Scan stays I/O trampoline (window, prefetch, SST decode). Not this RFC's
data-fate axis.

## handler_loc

0171 freeze on HEAD `ee6f9be3`: `handler_loc=102903`. This change moves
archive lookup out of `db.rs` so unique-caller loc drops on a HEAD-shaped
kernel set. Live dirty-tree number is the residuals stamp (extra concurrent
callers). `glue.db_rs_extracted` stays false. `never_floor` unchanged.
