# F47 — failed 2PC finish + heal/reopen majority-installs the TX

**Status:** closed (2026-08-14) — AE persist fail-closed + coordinated `TxnRevert` on the raft log + abort fence kept as defense
**Crate:** `pedradb-store`
**Class:** I-TX-2 / I-MAJ (client `Err`, later majority has the new value)
**Test:** `fail_after_mid_2pc_restores_preimage` (`crates/pedradb-store/tests/montanha_dst_primitives.rs`)

## What the soak showed

`FailingEnv` writes dead on a majority of nodes **during `tx_finish`**:

1. `tx_finish` returns `Err` (disk dead on 2/3).
2. At that instant there is **no** majority with the new user value.
3. Heal disks, drop the process (no `tx_cancel`), reopen, elect → the **new** value is in majority.

The client heard “failed”. After restart the cluster installed the TX.

## Root cause (not “ Pedra lost a preimage”)

Follower `AppendEntries` swallowed `persist_log_db` (`let _ = …`) and still replied `success: true`. The leader counted those acks, advanced `commit`, and (when apply on a dead disk then failed) returned `Err` **without** discarding — `commit_now >= idx`.

On reopen the leader’s durable log still had a **committed** `TxnCommit`. Election catch-up applied it to the healed majority.

A local `txn_status=abort` fence (first mitigation) is not a raft decision:

- it is a Pedra `put` that dies with the same disk;
- `apply_txn_revert` used to **delete** status when pairs were gone;
- install-snapshot wipes `\0store/txn/*` and copies **user** keys from the leader, bypassing `apply_txn_commit`.

## Fix

1. **AE persist fail-closed.** If `persist_log_db` fails, roll back the in-memory suffix and reply `success: false`. No commit advance on a non-durable majority.
2. **Coordinated revert.** `tx_finish` tracks ranges whose `TxnCommit` already majority-committed. On a later range failure those ranges get a `TxnRevert` **proposed on the same raft log**, then local preimage restore.
3. **Abort fence stays.** `apply_txn_commit` treats `status=abort` as revert; reopen recovery still fences leftover preimages. Revert no longer deletes an abort status. Snapshot install no longer deletes abort status.

## Residual (honest)

If a range *truly* majority-committed `TxnCommit` and the same majority disks stay dead, we cannot majority-commit `TxnRevert` either. The client already has `Err`. After heal, abort fence + reopen apply must hold. We do **not** claim that a partition that never heals can finish 2PC abort; we claim heal+reopen does not install a client-failed TX.

## Not claimed

Field peer with FDB 2PC. This is the lab store’s I-TX-2 hole under `FailingEnv`.
