# PoWER write preconditions → `wal_commit_plan`

**Date:** 2026-09-08
**Primary source:** LeBlanc et al., *PoWER Never Corrupts*, OSDI 2025. PDF already in
`findings/2026-09-07-capybarakv-unverified-crate/osdi25-leblanc.pdf`.

PoWER encodes crash consistency as **preconditions on writes**: a later step
(publish / apply / Ok) is illegal unless the durable prefix is already the
barrier. That is the script leftover in `commit_ops_with`: append then maybe
`sync_data` then apply then Ok lived in glue, so Verus on
`wal_sync_required` / `fence_on_sync_fail` could stay green while the handler
applied first.

## Used this turn

`wal_commit_plan(need_sync, sync_failed)` is the production term
`commit_ops_with` matches. Apply/Ok is not a variant when a required sync
failed (`AppendSyncFence`). AS-IS still applies after a failed sync.

## Not claimed

`Env::sync_data` persists. Media. Dump of `db.rs`. `∀π`.
