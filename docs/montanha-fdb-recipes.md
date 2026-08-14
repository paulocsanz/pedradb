# Montanha FDB design recipes (Phase 0 bug-hunt)

**Crate:** `montanha-fdb-recipes`  
**Upstream patterns:** [FoundationDB Design Recipes](https://apple.github.io/foundationdb/design-recipes.html)

## Goal

Exercise Montanha the way FDB layers do — **tables, secondary indexes, queues, multimaps, priority queues** — so SI/OCC/range bugs surface early.

Not a Record Layer clone. Not wire-compatible FDB.

## Recipes shipped

| Recipe | Type | Regression focus |
|--------|------|------------------|
| Subspace | prefix pack | key layout |
| Tables | sparse cells | multi-key TX |
| Simple indexes | primary + zip index | atomic index+row; concurrent Conflict |
| Queues | FIFO | concurrent pop serialization |
| Multimaps | multi-value keys | range scan |
| Priority queues | min pop/peek | ordered range + tombstones |

## Known Montanha delta vs FDB

- **`clear` is real Pedra delete** (empty payload on the wire ⇒ `BatchOp::delete` on apply). Recipes still treat empty as tombstone in range helpers for safety.
- **SI hist** is written on the Raft apply path (`si_gen` in Put/Batch log entries + `\0store/hist/`), not only as a side-channel local put.
- Index presence may still use `\x01` for clarity vs empty.

## Run

```bash
cargo test -p montanha-fdb-recipes
```

## Next phases

1. ~~fdb-compat / bindingtester subset~~ + **1b range/multi-key** — done  
2. ~~etcd-shaped multiproc workloads~~ — done (freeze, not full wire)  
3. ~~Record Layer seed~~ — done; full Java RL / SQL planner still long-horizon  
4. Optional: TCP etcd wire, full bindingtester
