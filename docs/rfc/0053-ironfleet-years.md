# RFC-0053: Formalização à escala IronFleet (anos)

**Status:** done (P0–P2 Y1 + Y2 + Y3 shipped; π/VerusSync não disparado — RFC-0051 sem dente in-tree, estado gravado)  
**Updated:** 2026-08-23  
**Orçamento:** ~3.7 pessoa-anos aceites (IronFleet SOSP’15 §7 — metodologia + IronRSL + IronKV)  
**Método que já corre:** `../determinismo/rfcs/0002-formalizacao-mista-ate-prova.md` (P0–P39 shipped; **P40** aberto)  
**Canónico:** `../determinismo/reports/formalizacao-e-prova-2026-08-14.md`  
**Estratégias:** [`../formal-verification-strategies.md`](../formal-verification-strategies.md)  
**DST / π / caixas:** [RFC-0050](0050-world-in-tree-fdb-determinism.md) · [RFC-0051](0051-beyond-fdb-sim-holes.md) · [RFC-0052](0052-dst-inside-boxes.md)  
**Produto:** piso 2× Rocks default (RFC-0041) **não** se troca por um engine 8× mais lento (VeriBetrKV)  
**O que “100%” exigiria:** [`../formal/one-hundred-percent.md`](../formal/one-hundred-percent.md)

---

## Em uma frase

Não vamos “provar o Pedra”. Vamos gastar os anos que o IronFleet gastou a fazer o **mesmo sanduíche**, no Rust que já produz:

```text
spec pequena (dicionário + crash) 
    ← protocolo (Raft/store) com redução “passo do host é atómico”
        ← kernels que o binário já chama  (vote, AE, commit, TX, WAL recover, …)
            ← Env / disco / net  = axiomas, DST para sempre
```

O FDB não tem este andar. Nós já temos dezenas de kernels. Este RFC é **compor e alargar** até um TCB nomeado, não reescrever em Dafny.

---

## Background

### O que os 3.7 anos *compraram* (números, não folheto)

| Sistema | Impl | Prova | Rácio | Nota |
|---------|------|-------|-------|------|
| IronRSL (Paxos) | 5 114 linhas Dafny | 39 253 | 3.6 : 1 | safety **e** liveness; loop principal no TCB |
| IronKV | — | — | — | mesmo paper |
| VeriBetrKV (OSDI’20) | KV + disco crashy | spec 283 linhas | — | crash = transição IOSystem; **single-thread**; 8× mais lento que Rocks |
| Verus log (SOSP’24) | crate Rust | 3.9 : 1, 12 s | — | Azure Storage; crash = state machine, não lógica nova |

Eles verificaram **código escrito para o prover**. Recusaram o C++ existente. Pedra já fez o gesto inverso: o `if` saiu do handler para um `fn` que **produção chama**. Isso é o ponto de partida, não o ano 3.

### O que Pedra já tem (não recomeçar)

- Kernels no binário: voto, AE, commit, propose-ack, lease, TX fence, prefix, WAL recover, bloom, pack, HTTP fail-closed, …  
- Verus ∀ no gémeo; Lean 2ª máquina no voto; Aeneas extract `vote_kernel.rs` LAKE-OK; Stateright no mesmo `fn`.  
- `Env` / crash como transição (Hance: não inventar Crash-Hoare).  
- **P40 ainda aberto:** `iff` no termo extraído `VoteKernel.vote_decision` (modelar `Option::eq`).

### Pain / why now

1. A nota de estratégias dizia “não começar IronFleet sem camada de protocolo e redução”. O orçamento de anos **foi aceite**. Falta o mapa, senão cada sessão tira mais um if HTTP (F79–F105) e chama-lhe “provar o Pedra”.  
2. Sem TCB escrito, ninguém sabe o que a prova *não* cobre (loop, `ConcurrentDb`, fsync).  
3. VeriBetrKV pagou 8× vs Rocks. O piso Pedra é **2× Rocks default**. Caminho = alargar kernels no engine actual, **não** um KV Dafny.

### Tese (não negociar)

```text
Ano a ano:
  1. todo kernel existente tem 2ª máquina (Aeneas ou Lean com drift check)
     + spec de crash do dicionário (acked prefix sobrevive)
  2. callers de Raft (persist-before-grant, AE ack) refinados no Verus;
     composição Stateright vote∧AE∧commit
  3. host loop (handle_request_vote / handle_ae) refina os kernels;
     liveness bounded; ConcurrentDb continua FORA do TCB
     (redução ou VerusSync é o último terço, RFC-0051)

TCB permanente: rustc, Verus/solvers, spec, loop de I/O, Env, CRC.
Axioma permanente: persist Ok|Err atómico; se o OS mentir, DST/TCG (0052).
```

Frase permitida no fim dos anos (a mesma de 2026-08-14 §8):

> Kernel K ⊨ spec S; Verus/Lean aceitaram. Relativo a axiomas A. Mutar K parte S e a máquina recusa. O dicionário, em crash, não reverte para além do último sync **nos caminhos cobertos pelos kernels**.

Frase **proibida** mesmo daqui a 3.7 anos: “não há bugs no Pedra.”

---

## Problems This Solves

- **Problem:** kernels cresceram em largura (HTTP) em vez de *composição*.  
- **Problem:** gémeo Verus ≠ termo Aeneas; P40 ainda é o buraco da 2ª máquina.  
- **Problem:** sem spec de crash estilo VeriBetrKV, “WAL recover verificado” não liga a `put` acked.  
- **Problem:** `ConcurrentDb` no TCB no dia um rebenta o orçamento (IronFleet era 1 thread).

---

## Proposed Solution

Três gestos, repetidos:

1. **Inventário TCB** — o que está dentro da prova, o que é axioma, o que é DST.  
2. **Refinamento** — handler = kernel + persist; o Verus prova o caller, não só o `if`.  
3. **Composição** — Stateright/Verus sobre *vários* kernels no mesmo modelo; spec de dicionário+crash no topo.

Horizonte (não são slices P0 — são anos; cada ano fecha teoremas nomeados):

| Ano | Entrega | Fora |
|-----|---------|------|
| **Y1** | P40 + 2ª máquina nos kernels de *destino de dados* (voto/AE/commit/TX/WAL/lease, não mais HTTP) + spec crash do dicionário com dentes DST | `db.rs` ∀; liveness; threads |
| **Y2** | `handle_request_vote` / `handle_append_entries` como refinamento Verus dos kernels; Stateright composto; apply do store | `ConcurrentDb`; geo |
| **Y3** | Dicionário+crash no path **single-thread** `Db` (VeriBetrKV); liveness bounded (quórum vivo); π/VerusSync só se 0051 tiver dente | provar o Linux; reescrever em Dafny |

---

## Delivery slices (mandatory)

Primeira vaga. Sozinha já muda o TCB: 2ª máquina no voto extraído + spec de crash escrita + inventário que o CI recusa se o handler deixar de chamar o kernel.

### P0 — TCB escrito + P40 + crash spec

- [x] **P0.1** Este RFC + TCB (§ abaixo) + horizonte Y1–Y3 — status: `done`  
- [x] **P0.2** P40.1: `iff` no termo Aeneas `VoteKernel.vote_decision` (sem axioma `Option::eq`) — status: `done` (`Vote.lean` `vote_decision_iff`; `Option::eq` is a match `def`; `pedra_formal.py` recusa o axioma)  
- [x] **P0.3** Página `docs/formal/crash-dictionary.md`: spec VeriBetrKV-shaped (“após crash, o mapa visível ⊇ prefixo acked”; CRC no TCB); ligar testes DST que já existem (`crash_after_sync`, silent_wrong) como **dentes da spec**, não como prova — status: `done`  
- [x] **P0.4** Inventário CI: `scripts/pedra_formal.sh --ci` falha se `handle_request_vote` / `rpc_request_vote` deixar de chamar `vote_decision` (lint de caller que já existe, torná-lo fail-closed e listar os kernels “destino de dados”) — status: `done` (`data_fate` + `handlers` no catalog)

### P1 — refinamento do caller + composição bounded

- [x] **P1.1** Verus no protocolo persist-before-grant (F15 caller), não só em `vote_decision` — status: `done` (`grant_after_persist` no handler + twin `ensures g ==> persist Ok`; `3 verified`)  
- [x] **P1.2** Stateright **composto**: vote ∧ AE ∧ `may_commit_at` no mesmo modelo; mutante AS-IS ainda dói — status: `done` (`tests/compose_model.rs`)  
- [x] **P1.3** `wal_recover` Verus ligado à spec P0.3 (torn tail / CRC) com mutante “EOF silencioso”) — status: `done` (`ZeroHeaderTail` + `lemma_as_is_zero_header_silent_eof`; `13 verified`)

### P2 — fechar Y1 sem abrir Y3

- [x] **P2.1** Segunda máquina (Aeneas ou Lean+drift) em `ae_entry_action` e `commit` recover — status: `done` (`Ae.lean` / `Commit.lean` over Aeneas extracts; `scripts/lean_ae_commit.sh`; SOURCE.ae / SOURCE.commit sha256)  
- [x] **P2.2** Relatório Y1: LOC impl/prova, lista TCB, o que DST ainda cobre; **sem** claim de dicionário ∀ — status: `done` (`docs/formal/y1-report.md`)  
- [x] **P2.3** Explicitar no TCB: `ConcurrentDb` e o loop TCP **fora** até Y3 / RFC-0051 — status: `done` (TCB abaixo + Y1 report)

### Y2 — refinamento do caller AE + apply do store

- [x] **Y2.1** AE caller (persist-then-ack) como refinamento Verus nomeado: `lemma_success_reply_only_after_persist` no twin `ae_ack_success` (`3 verified`); handlers AE (`handle_append_entries`, `rpc_append_entries`) fail-closed no catalog — status: `done`  
- [x] **Y2.2** Apply do store como kernel puro (`apply_kernel.rs::apply_advance`, chamado por `apply_committed`) + twin Verus (`lemma_apply_only_contiguous_committed_prefix`, `3 verified`); `compose_model` estendido com apply ∧ reopen: FIXED segura Inv-apply-contiguous / Inv-applied-le-commit e os 4 mutantes AS-IS (vote, AE, commit, apply) dóem — status: `done` (`tests/compose_model.rs`, 6 testes)  
- [x] **Y2.3** Redução Y2 no TCB (abaixo): “um passo de host = ler inputs → kernel → persist → outputs; interleaving de hosts = Stateright” — status: `done` (TCB v2)  

### Y3 — crash dictionary no reopen + liveness bounded

- [x] **Y3.1** Reopen do `Db` como kernel puro (`wal/reopen_kernel.rs::reopen_outcome`, chamado nos 4 braços de dano de `open_with_env`) + twin Verus com lemmas nomeados ligando recover→reopen (`lemma_recover_failstop_routes_to_damage`, `lemma_damaged_reopen_never_silent`, `lemma_fail_closed_refuses_damage`, `lemma_clean_reopen_serves_all`, mutante `lemma_mutant_swallows_damage`; `6 verified`) — status: `done`  
- [x] **Y3.2** Crash-dictionary estendido ao reopen (seção “Reopen outcomes” com a tabela de lemmas); dentes DST verdes — status: `done` (`docs/formal/crash-dictionary.md`)  
- [x] **Y3.3** Liveness bounded sob o **axioma quórum-vivo** (axioma explícito no modelo e no TCB, nunca teorema): `bounded_liveness_under_quorum_alive_axiom` — progresso grant+commit+apply em ≤ 4 passos no sub-modelo quorum-alive — status: `done` (`tests/compose_model.rs`)  
- [x] **Y3.4** π/VerusSync: condição gravada — **não disparado** (RFC-0051 segue `draft`, sem dente PCT in-tree; `ConcurrentDb` permanece fora do TCB por desígnio). Estado gravado, não falha — status: `done` (registo; disparo futuro = RFC-0051 P0 aterrar)  

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + TCB + horizonte Y1–Y3 | done | este ficheiro | 2026-08-23 |
| P0.2 | p0 | P40 iff no extract Aeneas do voto | done | Vote.lean + VoteKernel Option.eq def | 2026-08-23 |
| P0.3 | p0 | Spec crash-dictionary + dentes DST | done | docs/formal/crash-dictionary.md | 2026-08-23 |
| P0.4 | p0 | CI fail-closed se caller largar o kernel | done | catalog data_fate + handlers | 2026-08-23 |
| P1.1 | p1 | Verus persist-before-grant (caller) | done | grant_after_persist handler + twin | 2026-08-23 |
| P1.2 | p1 | Stateright vote∧AE∧commit | done | tests/compose_model.rs | 2026-08-23 |
| P1.3 | p1 | wal_recover ⊨ crash spec | done | wal_recover ZeroHeaderTail lemmas | 2026-08-23 |
| P2.1 | p2 | 2ª máquina AE + commit recover | done | Ae.lean + Commit.lean extracts | 2026-08-23 |
| P2.2 | p2 | Relatório Y1 (LOC, TCB, DST) | done | docs/formal/y1-report.md | 2026-08-23 |
| P2.3 | p2 | ConcurrentDb/TCP fora do TCB até Y3 | done | TCB v1 + y1-report | 2026-08-23 |
| Y2.1 | y2 | AE caller persist-then-ack refinado no Verus | done | verus/ae_ack_success.rs + catalog handlers AE | 2026-08-23 |
| Y2.2 | y2 | Apply do store como kernel + composição Stateright | done | apply_kernel.rs + compose_model (6 testes) | 2026-08-23 |
| Y2.3 | y2 | Redução host-step no TCB | done | TCB v2 | 2026-08-23 |
| Y3.1 | y3 | Reopen do Db como kernel + lemmas recover→reopen | done | wal/reopen_kernel.rs + verus/reopen_outcome.rs | 2026-08-23 |
| Y3.2 | y3 | Crash-dictionary: seção reopen | done | docs/formal/crash-dictionary.md | 2026-08-23 |
| Y3.3 | y3 | Liveness bounded sob axioma quórum-vivo | done | compose_model bounded_liveness_under_quorum_alive_axiom | 2026-08-23 |
| Y3.4 | y3 | π/VerusSync: condição gravada (não disparado) | done | registo neste RFC (RFC-0051 draft) | 2026-08-23 |

---

## TCB (v2 — Y2 reduction shipped; engine-kernel delta from RFC-0056 P0)

**Dentro da prova (hoje):** kernels puros listados em `crates/*/verus/` e `formal/aeneas/` (voto, AE, commit, apply-loop, TX, lease, prefix, WAL recover, …) **e** os callers Raft que os refinam (`handle_request_vote_with_persist` via `grant_after_persist`; `handle_append_entries` via `ae_entry_action` + `ae_ack_success`; `apply_committed` via `apply_advance`). **Delta RFC-0056 P0 (2026-08-23):** também dentro — as decisões LSM do engine (`manifest_kernel::sst_recover_action`/`first_install_action` chamadas por `recover_ssts`; `flush_kernel::flush_plan`/`wal_rotate_decision` chamadas por `flush`/`try_rotate_wal`/`ensure_wal_rotated_for_gc`; `compact_kernel::compact_pick` chamada por `compact_with_ssts_only` e `point_version_fate`/`lone_tombstone_fate` chamadas por `merge::gc_snapshot_safe`) — twins `manifest_recover`/`flush_decision`/`compact_decision` (29 verified, 0 errors, sem `sorry`; mutantes com dentes; relatório `docs/formal/p0-engine-report.md`). Bytes, fsync ordering e `ConcurrentDb` continuam caller+axioma.

**Axiomas (nunca teorema):** `persist` Ok\|Err atómico; CRC32C não forjável (como VeriBetrKV); relógio monótono se injectado; **quórum vivo para liveness** (Y3: axioma explícito do modelo `compose_model.rs` — `bounded_liveness_under_quorum_alive_axiom`; nunca teorema).

**Fora até Y3 / RFC-0051 (P2.3):** `ConcurrentDb` (group-commit e threads OS) e o loop TCP/event de `montanha-tcp` / `pedra-raft-node`. Também fora: io_uring ring; libc/`fsync`; HTTP para além dos predicados já extraídos.

**Redução (Y2 — shipped):** um passo de host = “ler inputs → kernel → persist → outputs”. Concretamente:

- `handle_request_vote_with_persist`: inputs (termo, voto, log) → `vote_decision` → persist (`PersistOutcome`) → `grant_after_persist` decide o grant na resposta. Twin Verus: `g ==> persist Ok`.
- `handle_append_entries`: inputs (prev-log, entradas) → `ae_prev_log_ok` / `ae_entry_action` (mutação do log) → `persist_log` → `ae_ack_success` decide o `success` da resposta. Twin Verus (Y2): `lemma_success_reply_only_after_persist` — success ∧ dirty ⇒ persist Ok.
- `apply_committed` (apply do store): inputs (`last_applied`, `commit`, presença da entrada) → `apply_advance` (Done/Stop/Apply) → apply na store → `last_applied += 1`. Twin Verus: `lemma_apply_only_contiguous_committed_prefix`.

Interleaving de hosts = Stateright (`tests/compose_model.rs`: vote ∧ AE ∧ commit ∧ apply ∧ reopen no mesmo modelo sobre os kernels de produção). Interleaving de threads **dentro** do host = RFC-0051, não este RFC, até Y3.

---

## Acceptance Criteria

### Tests

**P0**

- Lean/Aeneas: `vote_decision_iff` no extract, sem `sorry` e sem axioma `Option::eq` (P0.2).  
- Página crash-dictionary existe e aponta ≥2 testes DST existentes como dentes.  
- `pedra_formal.sh --ci` vermelho num mutante que inlinie o voto outra vez.

**P1**

- Verus verde no caller F15 (`grant_after_persist` `ensures g ==> persist Ok`; `3 verified`); handler chama o kernel.  
- Stateright composto (`compose_model`): FIXED 4 invariantes; AS-IS vote/AE/commit descobrem Inv-vote-once / F16 / F23|F11.  
- Mutante WAL ZeroHeaderTail = EOF silencioso: `lemma_as_is_zero_header_silent_eof`; CRC fresco fail-stop (`13 verified`).

**P2**

- Relatório Y1 no `docs/formal/y1-report.md` com rácio prova:código (≪ 3.6; sem dicionário ∀).  
- TCB v1 lista `ConcurrentDb` e o loop TCP **fora até Y3 / RFC-0051**.  
- Lean `Ae` + `Commit` sobre extracts Aeneas; `lake build Ae Commit` exit 0; sem `sorry` nos teoremas.

**Y2**

- Twin AE caller: `lemma_success_reply_only_after_persist` (`3 verified, 0 errors`, 2 runs, sem `sorry`); `handle_append_entries` chama `ae_ack_success`.  
- Twin apply: `lemma_apply_only_contiguous_committed_prefix` (`3 verified, 0 errors`, sem `sorry`); `apply_committed` chama `apply_advance`.  
- `compose_model`: FIXED 6 invariantes (+2 apply) e 2 non-vacuity; AS-IS vote/AE/commit/apply descobrem os contraexemplos.  
- Catalog fail-closed para handlers AE e apply (o lint vermelho se o handler sumir — testado negativo).

**Y3**

- Twin reopen: 5 lemmas nomeados ligando recover→reopen (`6 verified, 0 errors`, 2 runs, sem `sorry`); os 4 braços de dano de `open_with_env` chamam `reopen_outcome`.  
- Dentes DST do crash-dictionary verdes (crash_after_sync, truncate_tail, crc_fail_stops, silent_wrong gate, core reopen).  
- Liveness bounded: `bounded_liveness_under_quorum_alive_axiom` verde; o axioma quórum-vivo está declarado como **axioma** no modelo e no TCB (nunca teorema).  
- π/VerusSync: registado como **não disparado** (RFC-0051 sem dente in-tree).

### Telemetry / Analytics

- Contagem `verified` dos scripts `verus_*.sh`.  
- Tempo de `pedra_formal.sh --ci`.  
- Nenhum p99. Prova não é bench (RFC-0041 intacto).

### Documentation

- Status na mesma mudança que o código.  
- Esta página é o mapa de anos; RFC-0002 continua o ritual *por if*.  
- [`../formal-verification-strategies.md`](../formal-verification-strategies.md) aponta para aqui quando disser “IronFleet-scale”.

### Screenshots

- backend-only (`1 verified, 0 errors` / LAKE-OK).

### Claims permitidos

| Depois de | Permitido | Proibido |
|-----------|-----------|----------|
| P0 done | “Extract do voto prova o iff; TCB escrito; crash spec tem dentes DST” | “Pedra verificado” |
| Y1 (P2) | “Kernels de destino de dados têm 2ª máquina; WAL recover ⊨ spec de torn/CRC” | “dicionário ∀”; “mais correcto que FDB em campo” |
| Y3 | Frase do § tese (caminhos cobertos) | “não há bugs”; “fsync provado” |

---

## Out of scope

- Reescrita Dafny / Verdi-extract-OCaml / Coq do `db.rs`.  
- Abrir o TCB a `ConcurrentDb` no P0–P2.  
- Mais predicados HTTP como prioridade Y1.  
- Traçar 8× mais lento “porque VeriBetrKV”. Piso 2× Rocks default mantém-se.  
- Provar o OS (RFC-0052).  
- L29/L30.  
- Antithesis.

---

## Relação

| Doc | Papel |
|-----|--------|
| RFC-0002 determinismo | ritual por if; este RFC é o *orçamento de anos* |
| P40 NEXT.md | = P0.2 |
| RFC-0050/51/52 | andares 1 (sim / π / OS); este é andar 3 composto |
| VeriBetrKV | spec de crash a copiar; performance **não** |
| IronFleet | sanduíche e rácio; língua **não** (ficamos em Rust+Verus) |

---

## Como actualizar este doc

1. Ship slice → checkbox + Status.  
2. Novo kernel só entra no TCB se produção o chama **e** o inventário P0.4 o lista.  
3. REAL DST depois de um kernel provado → ou axioma mentiu, ou caller não refina, ou twin drift — nunca “a prova está errada e avançamos”.  
4. Fim de ano → P2.2-style relatório, mesmo que Y1 escorregue.
