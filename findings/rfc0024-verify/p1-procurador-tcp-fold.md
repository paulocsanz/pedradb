# P1 Procurador TCP/UDP request path is fold-only

**Date:** 2026-08-15

## Hole

TCP accept / UDP recv called `get_route` / `get_upstream` (SQL) on every
packet. HTTPS already had a LocalApplied fold after warm; TCP/UDP did not.

## Close

`PortProxy` holds a `BackendFold` (`DashMap` by route id and by upstream
id). Seed at bind / `create_upstream` / `RouteTargetsUpdated` (SQL on the
apply path). `run_tcp_listener` / `run_udp_listener` call
`lookup_tcp_backends` only. Miss = drop, not a query.

## Evidence

- `cargo test -p procurador --lib backend_fold_is_local` — ok
- Request-path grep: accept/recv has no `get_route` / `get_upstream`.

## Still true

- Seed is SQL. That is apply, not accept.
- Tests that never seed still miss the fold (no silent SQL fallback).
- `CachedRoute` still has no id; HTTPS uses the domain moka fold, not this
  map. `FoldRouteTable` in `fold_read.rs` is unused bytes.
