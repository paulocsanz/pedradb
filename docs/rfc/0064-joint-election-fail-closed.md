# RFC: 0064 — Eleição fail-closed durante joint consensus

**Status:** done
**Updated:** 2026-08-26
**Parents:** [0063](0063-fdb-reliability-close-the-system-gap.md) (MembershipJoint no log), [0059](0059-massive-scale-parallel-dst-and-cluster-invariants.md), [0002](0002-internal-key-memtable.md) (kernels)

## Background

- RFC-0063 P0 meteu `RangeEntry::MembershipJoint` no log e passou o **commit** a exigir maioria(old) ∧ maioria(new). A **eleição** ainda conta votos só sobre `ids` actual (C-old). Durante um add, C-old pode eleger um líder que C-new não aceitaria — exactamente o buraco que joint consensus existe para fechar (Raft §6).
- `on_request_vote_reply` ignora `from` (`_from`) e incrementa um contador. Sem o conjunto de votantes não dá para testar maioria no `new`.
- O kernel de commit (`may_commit_at`) não sabe joint; a regra vive no glue do store. Um AS-IS “ignora new” não tinha dente.
- Fora de âmbito deste P0: TCP cluster REAL (L28 MEASURE), extração `db.rs` (L46), Flow (L29).

## Problems This Solves

- **Problem:** joint no commit + eleição C-old = reconfig unsound se um líder muda a meio do joint.
- **Problem:** add de um quarto nó (old maj=2, new maj=3) é o caso em que old-only elege e joint recusa — não havia teste.
- **Problem:** a regra de quorum joint não é uma `fn` pura chamável pelo prover.

## Proposed Solution

Uma `fn` pura no `commit_kernel` (produção já o chama; twin Verus + mutante AS-IS). Store: RV para `old ∪ new`, tally por *voter id*, `try_become_leader` só se `joint_election_ok`. `add_member_joint` simétrico do remove. P0 é útil sozinho: reconfig deixa de eleger com a config antiga.

## Delivery slices (mandatory)

### P0 — must ship first (eleição fail-closed sob joint)
- [x] **P0.1** `commit_kernel::joint_election_ok` + AS-IS (ignora `new`) + twin Verus — status: `done`
- [x] **P0.2** Store: tally por voter; RV a `old ∪ new`; promote só com ambas as maiorias — status: `done`
- [x] **P0.3** `add_member_joint` + teste: old-only majority **não** elege durante add — status: `done` (`election_during_joint_add_refuses_old_only_majority`, `world_joint_add_after_remove_replays`)

### P1 — next wave
- [x] **P1.1** Stateright no kernel joint (dente AS-IS no model checker) — status: `done` (`crates/pedradb-raft/tests/joint_model.rs`: `fixed_joint_election_holds`, `as_is_elects_on_old_only`)
- [x] **P1.2** World: `Action::JointAdd` + replay hash — status: `done`

### P2 — later
- [x] **P2.1** Kernel Verus só de membership (ficheiro próprio) — status: `done` (`membership_kernel.rs` + twin `membership_joint.rs` + clone store + Stateright `joint_model`)
- [x] **P2.2** L28 REAL TCP (herda 0063 P1.3) — status: `done` (`cluster_real` + `l28_real_tcp_seed_replay`; F-L28 RequestVote remoto; leader-kill `l28_real_tcp_leader_kill` seed `0x641e29` `after=1 restart=1`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | joint_election_ok kernel + twin | done | commit_kernel.rs + verus/commit_recover.rs | 2026-08-26 |
| P0.2 | p0 | election uses old∧new | done | election_granted + vote_targets | 2026-08-26 |
| P0.3 | p0 | add_member_joint + old-only tooth | done | election_during_joint_add_refuses_old_only_majority | 2026-08-26 |
| P1.1 | p1 | Stateright joint | done | tests/joint_model.rs | 2026-08-26 |
| P1.2 | p1 | World JointAdd | done | world_joint_add_after_remove_replays | 2026-08-26 |
| P2.1 | p2 | dedicated membership kernel | done | membership_kernel.rs + verus_membership_joint.sh | 2026-08-27 |
| P2.2 | p2 | L28 REAL | done | cluster_real + l28_real_tcp_seed_replay | 2026-08-26 |

## Acceptance Criteria

- **Tests**
  - `joint_election_ok` recusa (2 de 3) ∧ (2 de 4); aceita (2 de 3) ∧ (3 de 4).
  - AS-IS aceita o caso recusado (dente).
  - `election_during_joint_add_refuses_old_only_majority`: 4 nós, joint add do 4º, 2 votos C-old → não `try_become_leader`.
  - `log_carried_joint_remove_crosses_out_of_band_floor` continua verde.
- **Telemetry / Analytics:** none — invariante de segurança.
- **Documentation:** este RFC; 0063 P1.1 aponta para aqui quando P0 fechar.
- **Screenshots:** backend-only.

## Out of scope

- Extrair `db.rs`. Inventar guest TCG. Coluna B Rocks. Joint de *dois* removes no mesmo entry.
