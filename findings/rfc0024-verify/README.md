# RFC-0024 verification

**Date:** 2026-08-14  
**Status:** implemented (`pedradb-fold` + Caixote seq_sync / live persist)

Gating logs live next to this file.

## Honesty

- Fold `get` is LocalApplied. Writes stay majority+fsync on Montanha.
- Fold consumers are not Raft voters.
- Federation `delta_only` → `sync_caixote_state_delta` (listed puts + `removed_*`
  tombstones). `sync_caixote_state_atomic` is first-ever (`fold_seq == 0`) or a
  rare 5-minute *session* backstop — not reopen, not every reconnect.
- Live path is `prepare_outbound_state` (RFC 0172 names). `FederationClient::new`
  loads `observed-fold.json` into `prev_*` / `state_seq` / `last_state_hash`.
  `connect_and_run` does **not** set `force_full_state` when `state_seq > 0`.
  Fingerprints + seq are written only after `tx.send` succeeds; a failed send
  restores the in-memory baseline (cursor-after-apply).
- Procurador request path still reads `RouteCache` (moka). `fold_from_log`
  incrementally updates that cache. A **warmed** miss still falls back to SQL
  (`get_route_by_domain`) — `FoldRouteTable` is not on the HTTPS/TCP/UDP lookup
  yet. That is the next live-path hole, not this persist slice.
- Intent resume (`last_intent_version` + `skip_full_intent`) is independent of
  observed persist.

## Evidence

- `p1-live-persist.log` — `observed_fold` + `outbound_state_tests` (20 ok)
- `p1-seq-sync.log` — `federation-api --lib seq_sync` (4 ok)

