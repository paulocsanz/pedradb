# RFC-0202 — sweep final: 6/6, gates GREEN no HEAD destacado

Data: 2026-09-11. HEAD final da ronda: o commit de fechamento deste
sweep (RFC 0202 `**Status:** done`). Promotions da ronda (uma por
commit):

| Slice | Commit | Conteúdo |
|-------|--------|----------|
| P0.1 | `7634cd77` | atom `rwlock_client_may_mutate` (GroupCommit.lean) — cap 98→97, floor_atom 33→34 |
| P0.2 | `85309685` | atom `occ_member_fate` (GroupCommit.lean) — cap 97→96, floor_atom 34→35 |
| P1.1 | `6bab9882` | ponte `wait_for_deadlock` 3 arestas (Locktab.lean) — cap 96→95, floor_atom 35→36, floor_extract 243→242, fronteira datada EXTRACT.md |
| P1.2 | `5f676964` | quinto close `bearer_token_from_value_fate_iff` (Auth.lean) — floor_close 4→5, residuals close 5→6 |
| P2.1+P2.2 | commit deste doc | RECUSA do escalonador registrada no RFC (nenhum flip) + sweep |

## Sweep (worktree destacado do HEAD final)

- `git worktree add --detach <tmp>/wt <HEAD>` → três gates:
  - depth-floor: GREEN — extract=242 (floor 242), registered ladder
    close=5 (floor 5) / atom=36 (floor 36), residuals close=6/atom=36
    == live 6/36, count=7 (floor 7, residual == live), data_fate=95≤95
  - product-floor: GREEN — D1=close R1=atom T1=atom C1=close,
    promoted=4≥floor 4
  - ledger: GREEN — 19 pointers resolvem, total=299 proof=266
    campaign=33
- Worktree removido após a captura (`git worktree remove --force`).

## Wrappers tocados — sorry 0

- `formal/aeneas/lean/GroupCommit.lean` (2 atoms): `rg -c sorry` = 0
- `formal/aeneas/lean/Locktab.lean` (3 pontes): 0
- `formal/aeneas/lean/Auth.lean` (close): 0
- `scripts/lean_extracts.sh --required`: ok (61 libs + 12 compose)

## Plantas DST (produção) verdes no HEAD

- `rwlock_client_may_mutate` — commit 7634cd77 (rocksdb-compat 1/1)
- `occ_member_fate` — commit 85309685 (1/1)
- `wait_for_deadlock_on_live_cycle_is_not_ok` — commit 6bab9882 (1/1)
- `bearer_token_from_value_on_live_http_is_not_ok` — commit 5f676964
  (pedradb-http 1/1)
- `claim_lock_interleavings_refused_after_put` (P2.1, recusa) —
  pedradb-core 1/1

## Admissions — recusadas no HEAD (nunca flipadas)

- `media_durable_admitted` — always false
  (`claim_media_durable_refused_after_fsync_ok`)
- `forall_schedules_admitted(3)` — false (0156 P1.1)
- `lock_interleavings_admitted` — always false
  (`claim_lock_interleavings_refused_after_put`; AS-IS admitiria)

## Escada no HEAD final

extract 242 / close 5 registrados (floor 5, residual 6 = 5 + 1 twin sem
extração) / atom 36 (floor 36) / count 7 / cap_data_fate 95.
