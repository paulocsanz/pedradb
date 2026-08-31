# RFC: 0158 — O termo só sobe com hard state durável (F125/F127 vira par do catálogo)

**Status:** in-progress
**Updated:** 2026-08-30
**Parents:** [0152](0152-live-queued-vote-ae-catalog-kernel.md), [0155](0155-silent-wrong-fail-closed.md)

**Residual:** este RFC não apaga linha nenhuma (`R-glue` segue; o handler continua TCB). A decisão vira kernel checável, o I/O fica no glue. `never_floor` intocado; `db_rs_extracted` segue `false`.

**Refused claims:** o par novo não é "garantia total", "sem bugs", "acabou" ou seL4. O gêmeo prova a fn pura relativa aos axiomas (PersistOutcome é axioma do Env); a planta prova o caminho inbound Queued num cenário, não ∀ traces; o clone pedradb-raft↔pedradb-store continua drift-trap humano (lint compara tokens, não semântica).

## Background

- `persist_hard_db` tem dois caminhos vivos no store. O do voto está congelado desde o 0152 (par `grant_persist`: `vote_kernel::grant_after_persist` + planta `grant_after_persist_on_live_queued_is_not_ok`). O do **termo** não: `durable_become_follower_if_newer` (`crates/pedradb-store/src/lib.rs:2605`, F125/F127) é cola inline em 6 sítios inbound (`on_request_vote`, `on_append_entries`, replies, …).
- A decisão que vive aí: termo recebido mais novo ⇒ `become_follower(novo)` + persist; persist `Err` ⇒ restaura termo/voto anteriores, força `Role::Follower`, limpa `leader_id` — o processo nunca age num termo que não está em disco.
- O board `/verificacao-next` de 2026-08-30 não achou cartoon, `absent`, buraco de freeze ou drift de `live_callers` — este era o único habitante classe D: bit de protocolo vivo sem fn de catálogo.
- O crate `pedradb-raft` mantém uma cópia idêntica de `vote_kernel.rs` (clone `vote_raft_store`; store não depende de pedradb-raft). Todo acréscimo entra nos dois lados com tokens idênticos.

## Problems This Solves

- **Problem:** a regra F125/F127 é checada só por revisão — nenhum kernel, gêmeo, AS-IS ou planta a nomeia; um refactor pode transformá-la no mutante (manter termo alto sem durabilidade) sem nada que pare vermelho.
- **Problem:** o passo do termo e o passo do voto são as duas metades do mesmo protocolo (F15/F125/F127); só uma metade tem três dentes.

## Proposed Solution

Extrair a decisão pura `durable_term_if_newer(termo_atual, termo_recebido, PersistOutcome) -> {Keep, Raised, Restored}` para `vote_kernel` (nas duas cópias do clone), ligar o handler vivo do store a ela, e registrar o par `durable_term` no catálogo com gêmeo Verus `close` + runner próprio + AS-IS + `live_callers` + planta inbound Queued com persist falhando pela costura `Env`. O AS-IS modela o erro silencioso: manter `Raised` mesmo com persist `Err`.

## Delivery slices (mandatory)

### P0 — o par existe e morde (shippable sozinho)

- [x] **P0.1** Kernel puro `durable_term_if_newer` + AS-IS `durable_term_if_newer_as_is` em `vote_kernel.rs` (pedradb-raft e pedradb-store, tokens idênticos) + testes unitários incluindo o dente do mutante — status: `done` (`Keep` em termo ≤ atual; `Raised` só com `Ok`; `Restored` com `Err`; AS-IS devolve `Raised` com `Err` — dente)
- [x] **P0.2** Gêmeo Verus `verus/durable_term.rs` (spec fechada + lema "termo sobrevivente só sobe com persist Ok") + runner `scripts/verus_durable_term.sh` + par `durable_term` no catálogo com `live_callers` no handler vivo `durable_become_follower_if_newer`; clone `vote_raft_store` passa a vigiar `durable_term_if_newer` — status: `done` (`verus_check.sh --all` 57/57; lint nomeia o par)
- [x] **P0.3** Planta `durable_term_rollback_on_live_queued_is_not_ok` no caminho inbound REAL (Queued `handle_inbound`, `FailingEnv` de falha única, RequestVote de termo mais novo): afirma no vivo a resposta deny, termo/voto restaurados, `Role::Follower`, `leader_id` limpo — o vivo bate com o kernel, o AS-IS admitiria o errado — status: `done` (`cargo test -p pedradb-store --lib durable_term_rollback_on_live_queued_is_not_ok` verde)

### P1 — próxima onda

- [ ] **P1.1** Alinhar `handle_request_vote_with_persist` (pedradb-raft) ao mesmo kernel — hoje ele sobe o termo em memória e só persiste o voto (formato AS-IS; legal porque produção passa pelo store, mas o dente do crate vive emprestado) — status: `todo`

### P2 — depois

- [ ] **P2.1** Guarda REAL TCP na família `l28_tcp_*` para o rollback durável (removed-replica + termo novo + persist falho no disco real) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | kernel + AS-IS + testes unitários nas duas cópias | done | este commit | 2026-08-30 |
| P0.2 | p0 | gêmeo + runner + par no catálogo + live_callers | done | este commit | 2026-08-30 |
| P0.3 | p0 | planta inbound Queued com persist falho | done | este commit | 2026-08-30 |
| P1.1 | p1 | alinhar handler do pedradb-raft ao kernel | todo | — | 2026-08-30 |
| P2.1 | p2 | guarda REAL TCP do rollback durável | todo | — | 2026-08-30 |

## Acceptance Criteria

- **Tests** (nomeados): `cargo test -p pedradb-raft` (unitários do kernel + dente do mutante); `cargo test -p pedradb-store --lib durable_term_rollback_on_live_queued_is_not_ok` (planta inbound); `bash scripts/formal/verus_durable_term.sh` (gêmeo); `python3 scripts/formal/pedra_formal.py --lint` nomeia `durable_term` com `0 gap, 0 fail` e freeze == live.
- **Telemetry / Analytics:** none — round de catálogo formal; nenhuma mudança de comportamento observável em produção (o wiring é equivalência passo-a-passo do mesmo handler).
- **Documentation:** este RFC; par `durable_term` no `catalog.json`; números de freeze no `residuals.json` recalculados no mesmo commit.
- **Screenshots:** backend-only.

## Out of scope

- Campanhas novas (PCT d, seeds TCP noturnas) — não pedidas.
- Extrair `db.rs`; mexer em `never_floor`; novos residuals.
- Semântica do handler do pedradb-raft (fica no P1.1, adiada de propósito).
- Trabalho de performance dos RFCs 0153/0154; benches.
