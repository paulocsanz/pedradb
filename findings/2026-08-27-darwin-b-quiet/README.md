# Darwin coluna B — quiet waiter

Still waiting. Last samples load1 8–11 / 17 users. Gate is **< 4.0** twice.

When it fires: 3 isolated rounds (`ycsb_a`, `deps_raftlog`, G1 vs
`FULL_SYNC=1` on the live `NNNNNN.log`). Compare takes the rocks JSON as
3rd arg (`ROCKS_PARITY_ALLOW_SYNC_PEER=1`). This file is overwritten with
the min/median table at the end.

Not a gate until `r1/` exists.
