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

- **`clear` stages empty values** rather than removing keys from range scans. Recipes treat **empty value as tombstone** (`is_live`). Index presence uses `\x01`, not FDB’s empty string.
- Future store work: true delete / range tombstones would simplify layers.

## Run

```bash
cargo test -p montanha-fdb-recipes
```

## Next phases

1. fdb-compat / bindingtester subset  
2. etcd-shaped multiproc workloads  
3. Record Layer shim (long)
