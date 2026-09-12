# RFC-0214 — Drenar a espinha de durabilidade: env ×6 + wal_state ×6 + cqe ×5 + write_ack ×3 no degrau átomo

**Status:** draft
**Data:** 2026-09-12
**Autoria:** agente grind (round 8), sucessora direta do RFC-0213
(catálogo ZERO `data_fate`)

## Contexto

O RFC-0213 fechou o catálogo: 292 pares, ZERO `data_fate` pendente —
os três blocos (membership 0211, cluster 0212, storage 0213) drenados
ao degrau átomo. A escada viva medida ao vivo no HEAD `7f1c7cf4`:
extract=156, close=6, atom=122, count=7, `cap_data_fate 0<=0` GREEN.

O próximo avanço significativo rumo ao seL4 não é um cap novo de
`data_fate` (zerado e congelado) — é SUBIR A ESCADA dos que ficaram:
156 pares cujo melhor artefato Lean ainda é um teorema de extrato
(valor concreto, sem ∀). Dentre eles, o bloco de maior valor de
produto é a **espinha de durabilidade** — o caminho que decide o que
sobrevive a um crash e quando o Ok é honesto:

- `wal_state_kernel.rs` ×6 — `wal_state`, `wal_append`, `wal_sync`,
  `wal_ack`, `wal_rotate`, `wal_acked_survives` (a máquina de estados
  do WAL: o que o ack garante);
- `env_crash_kernel.rs` ×6 — `env_crash`, `env_append`, `env_sync`,
  `env_barrier_floor`, `env_no_invented`, `env_honest_sync` (a costura
  Env: crash model e sync honesto);
- `cqe_kernel.rs` ×5 — `cqe_res`, `cqe_tags`, `cqe_leftover`,
  `cqe_submit`, `cqe_ring_refusal` (o anel de completions io_uring:
  a honestidade do fsync barrier);
- `write_ack_kernel.rs` ×3 — `write_ack_append`, `write_ack_barrier`,
  `write_ack_ack` (o ledger write→ack da coluna G1).

20 pares, todos com wrapper Lean já inscrito no LIBS
(`WalState`, `EnvCrash`, `Cqe`, `WriteAck`) e extrato carimbado —
zero buracos de inscrição nesta fatia. Wrappers existem e buildam;
o que falta é o degrau: cada `entry` com teorema iff-∀ sobre o corpo
extraído (o padrão `fate_iff` dos RFCs 0205–0213), promovido por
`promote_atom.py` com `floor_atom +1 / floor_extract −1` no mesmo
commit da planta DST.

## Meta mensurável

Escada final (se os 20 pousarem): `floor_atom 122→142`,
`floor_extract 156→136`, close=6 e `cap_data_fate 0<=0` imutáveis.
Vereditos datados (P1.2) ajustam o alvo SEM inventar gate — o caminho
previsto, não uma exceção. Contagem autoritativa: o gate
`check_depth_floor.py` no HEAD de cada promoção.

## Fatias (cadência: 1 promoção = 1 commit, teorema iff no wrapper,
planta DST verde ANTES do commit, gate GREEN no commit)

1. **P0.1** wal_state ×6: `wal_state`, `wal_append`, `wal_sync`,
   `wal_ack`, `wal_rotate`, `wal_acked_survives` (wrapper
   `WalState.lean`) — floor_atom 122→128, floor_extract 156→150 —
   status: `todo`

   — 1/6 `done`: `inv_wal_fate_iff`
   (WalState.lean; o desfecho de `inv_wal` é EXATAMENTE a conjunção
   Booleana `acked ⊆ synced ∧ synced ⊆ written`; perna citada
   `wal_inv_closed`, RFC-0191 P2.1; o as-is esquece o braço
   acked⊆synced — dente `inv_wal_as_is_dente`), floor_atom 122→123,
   floor_extract 156→155; planta DST verde
   (`wal_inv_on_live_recording_is_not_ok`, 1 passed)
2. **P0.2** env_crash ×6: `env_crash`, `env_append`, `env_sync`,
   `env_barrier_floor`, `env_no_invented`, `env_honest_sync` (wrapper
   `EnvCrash.lean`) — floor_atom 128→134, floor_extract 150→144 —
   status: `todo`
3. **P1.1** cqe ×5 + write_ack ×3: `cqe_res`, `cqe_tags`,
   `cqe_leftover`, `cqe_submit`, `cqe_ring_refusal` (wrapper
   `Cqe.lean`) + `write_ack_append`, `write_ack_barrier`,
   `write_ack_ack` (wrapper `WriteAck.lean`) — floor_atom 134→142,
   floor_extract 144→136 — status: `todo`
4. **P1.2** veredito datado dos medidos ausentes (SE houver: par
   cujo entry/planta não existe ou cuja classe é campanha/capacidade,
   ex. liar-campaign — recusa/aposentadoria datada SEM quebrar
   âncoras 0203/0204; nunca gate inventado) — status: `todo`
5. **P2.1** composição ∀ da espinha de durabilidade (env → wal →
   ack sobre atoms registrados) em NOVA compose lib (zero buracos;
   twins kernel/planta DST verdes; SEM registro TSV — razão datada em
   findings, não é par único) — status: `todo`
6. **P2.2** sweep final (worktree destacado DENTRO de `software/`,
   gates 3× GREEN — depth-floor, inventory-terminal, twin-contracts —
   + `test_proof_vs_campaign` ok, extracts `ok` com a contagem nova,
   sorry 0, capturas em findings, nota datada em EXTRACT.md:
   espinha de durabilidade no degrau átomo) + flip
   `**Status:** done` — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | wal_state ×6 no degrau átomo | doing | 1/6 | 2026-09-12 |
| P0.2 | p0 | env_crash ×6 no degrau átomo | todo | — | 2026-09-12 |
| P1.1 | p1 | cqe ×5 + write_ack ×3 no degrau átomo | todo | — | 2026-09-12 |
| P1.2 | p1 | Veredito datado dos medidos ausentes | todo | — | 2026-09-12 |
| P2.1 | p2 | Composição ∀ env→wal→ack + twins DST | todo | — | 2026-09-12 |
| P2.2 | p2 | Sweep final + nota EXTRACT.md + flip done | todo | — | 2026-09-12 |

## Critérios de aceite

- Cada promoção: teorema iff no wrapper (build `lake build <Lib>`
  verde ANTES do commit), planta DST verde ANTES do commit, cirurgia
  `promote_atom.py` (floor_atom +1, floor_extract −1, linha `atom`
  no `close_proofs.tsv`, `atom_reason` datado no catálogo,
  residuals), `check_depth_floor.py` GREEN no commit — 1 promoção =
  1 commit, `git show <c> -- <lean> | grep -c "^+theorem"` = 1.
- Nenhuma promoção pode: quebrar âncoras 0203/0204, registrar gate
  inventado, tocar arquivos não commitados da sessão paralela em
  `pedradb-core`/`wal` (checar `git status` por arquivo antes de cada
  commit; se um kernel alvo estiver sujo dela, adiar SÓ aquela
  promoção e seguir as demais).
- `cap_data_fate 0<=0` permanece GREEN e imutável — esta RFC não
  reabre cap de `data_fate`; a série que desce é `floor_extract`.
- Pares de classe campanha/capacidade (ex. liar-campaign) NÃO são
  promovidos à força: veredito datado no P1.2 com a recusa medida.
- P2.1 sem registro TSV (razão datada em findings: atravessa três+
  kernels).
- P2.2 exige: gates 3× GREEN + `test_proof_vs_campaign` ok no
  worktree destacado dentro de `software/`, extracts `ok` com a
  contagem nova, sorry 0, capturas em findings, nota datada em
  EXTRACT.md, `**Status:** done`.

## Fora de escopo (nomeado, não esquecido)

- O bloco `group_commit_kernel.rs` ×11 (inclui pares de campanha do
  escalonador — `forall_schedules`, `lock_interleavings` — que
  PERMANECEM `ok false` por decisão TCB): sucessora natural após esta
  RFC, com veredito de classe antes da meta numérica.
- Dump de `db.rs`/`concurrent.rs`; re-pin de Aeneas/charon sem widen
  medido; Montanha (congelada); qualquer cap novo de `data_fate`.
