# Layer freeze (RFC-0017 P2.3)

**Updated:** 2026-08-13

## Rule

For **cluster-facing** product proofs (lease / DCS / index batch / journal pins):

| Allowed | Forbidden (dual path) |
|---------|------------------------|
| `StoreCluster::dcs_create` / `dcs_cas` / `dcs_get_on` | Standalone `pedradb_dcs` Raft or etcd-shaped second product for the same lock |
| `StoreCluster::put` / `put_batch` / `commit_tx` | Writing “cluster meta” only to local Pedra without store majority |
| Multi-process smoke: process A write → process B reopen verify | In-process-only as the *only* proof for “multi-process durable” |

Local Pedra (`Db` / `ConcurrentDb` / `pedradb-lease` on one node) remains valid for **kernel** canaries (RFC-0020 lanes). It is **not** a substitute for Montanha multi-process durability.

## Proof in-tree

```bash
cargo test -p pedradb-store --test multiprocess_tx multi_process_dcs_layer_freeze
# smoke binary:
#   montanha-store-smoke dcs-layer <dir>
#   montanha-store-smoke dcs-layer-verify <dir>
```

Covers:

1. **DCS layer** — exclusive create + CAS + majority after OS-process reopen  
2. **App layer** — index-style `put_batch` row+idx on the **same** store  
3. **No dual consensus** — only StoreCluster multi-Raft

## Freeze checklist (for new layers)

- [ ] Uses only public store APIs (`dcs_*` / put / batch / commit_tx)  
- [ ] Multi-process write→verify exists  
- [ ] Does not introduce a second leader election for the same coordination object  
