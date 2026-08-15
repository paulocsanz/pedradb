# P1 HTTPS request path never SQL

**Date:** 2026-08-15

## Hole

`HttpsProxy` called `RouteCache::get()`. After a failed warm
(`warmed == false`) `get()` SQL-falls-back on every Host header. Cold
boot "start empty, refresh later" made the request path a Postgres
client.

`lookup_request_route` from an earlier close had been wiped.

## Close

- `lookup_request_route`: moka only. Miss = None.
- HTTPS uses that, not `get()`.
- Failed warm calls `mark_ready()` so even `get()` stays LocalApplied.
- SQL remains on warm / reconcile / NATS apply / admin `get()` before warm.

## Evidence

`cargo test -p procurador --lib lookup_request_route`
