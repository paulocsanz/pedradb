# RFC-0208 P2.2 — sweep final: worktree destacado, seam store/raft nomeado, 0208 fechado

Data: 2026-09-11. HEAD varrido: `98d7b960` (o último commit do 0208,
P2.1 2/2). Worktree destacado DENTRO de `software/`
(`/Users/paulo/software/pedradb-wt0208`, detach no HEAD) — caminho
relativo do backend Aeneas válido.

## Capturas (todas do worktree destacado)

```
GATE depth-floor: GREEN — extract=231 (floor 231), registered ladder
  close=6 (floor 6) / atom=47 (floor 47), residuals close=7/atom=47
  == live 7/47, count=7 (floor 7, residual 7 == live 7),
  data_fate=84<=84, handler_loc=112092 (series; TSV 111927)
GATE product-floor: GREEN — D1=close R1=atom T1=atom C1=close,
  promoted=4>=floor 4
GATE ledger: GREEN — 19 catalog pointers resolve, counts match
  (total=299 proof=266 campaign=33)

✔ [1928/1929] Built WalState (3.7s)
Build completed successfully (1929 jobs).
ok    lean extracts (61 libs + 19 compose)
```

Sorry 0 nos wrappers tocados no 0208 (grep no worktree):
`L28: 0 · Membership: 0 · Vote: 0 · Commit: 0 · ComposeStoreRaft: 0`.

As três admissions seguem ALWAYS false com recusas plantadas em
produção (`group_commit_kernel.rs`; plants
`forall_schedules_admitted_on_live_group_is_not_ok` /
`claim_lock_interleavings_refused_after_put` /
`claim_media_durable_refused_after_fsync_ok` em concurrent.rs e
db.rs).

## O que o 0208 deixou pago (8 atoms + composição, um commit cada)

| Slice | Atom / teorema | Commit |
|---|---|---|
| P0.1 | `grant_after_persist_fate_iff` (Vote.lean) | `7adc0cf1` |
| P0.2 | `propose_ack_ok_fate_iff` (Commit.lean) | `ef36c681` |
| P1.2 1/4 | `removed_steps_down_fate_iff` | `c3bfeb72` |
| P1.2 2/4 | `disk_membership_overrides_cli_fate_iff` | `9188ec08` |
| P1.2 3/4 | `high_water_at_least_fate_iff` | `08016c7d` |
| P1.2 4/4 | `joint_still_active_fate_iff` (ponte de specs) | `8837e2ad` |
| P2.1 1/2 | `l28_tcp_left_ok_fate_iff` (L28.lean) | `b5e15cb9` |
| P2.1 2/2 | `l28_tcp_hw_ok_fate_iff` (L28.lean) | `98d7b960` |

P1.1 (composição, `faec9c9d`): `election_grant_chain_fate` +
`recovery_fate_composed` em ComposeStoreRaft.lean (19º compose lib),
zero sorry, twins 7/7 — sem registro no TSV por não serem par único
do catálogo (razão datada em findings do P1.1).

Escada no fechamento (contra o início do 0208): extract 239→231,
close 6 (residual 7), atom 39→47, count 7, cap_data_fate 92→84 —
os alvos exatos do RFC. Plantas: DST 1/1 nos atoms P0/P1.2; TCP
REAIS nos 2 do P2.1 (235s + 232s).

## Seam store/raft (nota datada completa em formal/aeneas/EXTRACT.md)

- Kernels raft (vote/commit/membership) com ZERO `data_fate`
  pendente — medido ao vivo no HEAD.
- Cluster: 8/66 pagos pelo 0208; restante vivo nomeado = **58**
  (22 membership + 29 l28 + 6 txn + 1 compact_unleft). O "55" do
  texto do slice P2.2 subtraía o trio pago no 0205
  (`vote`/`recover_apply`/`recover_drop_orphan`), que JÁ estava
  fora dos 66 na contagem do board — correção datada no EXTRACT.md
  e em plan.md (Deviations).
