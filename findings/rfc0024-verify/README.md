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
  (This wiring was wiped from the tree and restored 2026-08-14.)
- Intent resume: register sends `intent_content_version` of the live
  cache (tombstones excluded). Incremental wall-clock is **not** stored.
  A timestamp left on disk self-heals. `skip_full_intent` is hash equality.
- Procurador HTTPS request path is **only** `lookup_request_route` — never
  `get()` / SQL, even if boot warm failed. Failed warm calls `mark_ready()`
  so the fold (possibly empty) is the SoR until refresh.
  TCP/UDP accept/recv: `lookup_tcp_backends` (seeded at bind / apply).
  TLS handshake already reads `get_sync` (local). SQL is warm / reconcile /
  `route_log` / NATS apply / seed. Intent resume is independent of observed persist.

## Evidence

- `p1-live-persist.log` — `observed_fold` + `outbound_state_tests` (20 ok)
- `p1-seq-sync.log` — `federation-api --lib seq_sync` (4 ok)
- `p1-procurador-fold.log` — fold_read + warmed get + nats nil + tcp backends (7 ok)
- `p1-procurador-upstream.log` — fold upstream + TCP backend from fold (12 ok incl. prior)
- `p1-procurador-https-fold.log` — HTTPS always fold; no SQL on !warmed (13 ok)
- `p1-procurador-tcp-fold.md` — TCP/UDP accept is BackendFold, not SQL
- `p1-intent-content-cursor.md` — register cursor is content hash; timestamp disk self-heals
- `p1-https-fold-only.md` — `lookup_request_route` restored; mark_ready on failed warm

