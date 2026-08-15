# RFC-0024 live persist cannot stay on disk

**Date:** 2026-08-14

## Hole (still true)

1. `FederationClient::new` / reconnect set `force_full_state = true` →
   host-kill+reopen full-dumps observed (`sync_caixote_state_atomic`).
2. Incremental IntentSync uses `intent_version = timestamp_millis`. If the
   agent persists every version, `skip_full_intent` never matches a content
   hash on reopen → desired dump every reconnect.
3. Fix: persist `observed-fold.json` after successful send; dump only if
   `fold_seq == 0`; persist `intent-fold.json` **only** on `full_sync`.

## What happened

A parallel writer on `/Users/paulo/software/caixote` deletes
`observed_fold.rs` and reverts `federation_client.rs` / proto / seq_sync
within minutes of each restore. This session restored the wiring twice;
both times the files vanished before a clean test list could see
`observed_fold::tests`.

Procurador `lookup_request_route` was also wiped.

## What would prove the hole closed

- `cargo test -p caixote-api --lib observed_fold` (persist + full_sync-only cursor)
- `cargo test -p caixote-api --lib persisted_cursor`
- `cargo test -p federation-api --lib matching_intent`
- Files still present after the test process exits
