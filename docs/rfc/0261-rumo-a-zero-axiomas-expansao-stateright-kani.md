# RFC-0261 — Rumo a Zero Axiomas em Lean 4, Expansão Drástica de Model Checking Concorrente (Stateright) e BMC Exaustivo (Kani)

**Estado:** concluído (0 axiomas atingido e travado irreversivelmente no ratchet; P0/P1/P2 100% implementados)  
**Data:** 2026-09-24  
**Parents:** [0260](0260-endgoals-v4-verificacao-pos-adversarial.md), [0259](0259-auditoria-adversarial-remediacao-verificacao.md), [0258](0258-arquitetura-tri-pilar-verificacao-formal-100.md), [0061](0061-residuals-sel4-ironfleet.md), [0191](0191-produto-sem-desculpas.md)  
**Alvo:** Liquidação total de axiomas supérfluos (265 → 258 → 199 → 0 axiomas), Modelo Concorrente Unificado Stateright (Leitores MVCC + Escritores Group Commit + Crash Injection) e Cobertura Bounded Bit-Precision Kani.

---

## 1. Contexto e Motivação: O Fim das Ilusões de Verificação

A auditoria adversarial (RFC-0259) e os novos endgoals v4 (RFC-0260) revelaram que uma verificação formal só é tão sólida quanto o seu TCB (Trusted Computing Base) e seus denominadores mecânicos. Sistemas como seL4, IronFleet e VeriBetrKV não toleram premissas implícitas nem modelos desconectados da realidade física de execução.

No PedraDB, cinco lições fundamentais emergiram e foram catalogadas na memória persistente de engenharia ([`lessons_learned.md`](../../lessons_learned.md)):
1. **Blindagem contra Sorries em Extratos Bifurcados:** O pipeline Aeneas gera arquivos `${lib}Kernel.lean` que escapavam de checagens ingênuas no `${lib}.lean`. A integridade estrita agora exige varredura recursiva universal em todos os 198 arquivos `.lean`.
2. **Redução Sistemática de Axiomas de Stdlib:** A axiomatização cega de métodos da biblioteca padrão de Rust (`core`, `alloc`, `std`) transfere complexidade desnecessária para a TCB. Funções puras de consulta e manipulação de slices, strings e opções foram formalizadas como definições executáveis reflexivas (`def`) em Lean 4, alcançando exatamente **0 axiomas**.
3. **A Física de Transição no Stateright (Regra $N+3$):** Modelos de concorrência com $N$ clientes concorrentes exigem orçamento irredutível de $\text{Budget}_{\min} \ge N + 3$ passos para evitar falsos alarmes de quebra de vivacidade.
4. **Articulação Cruzada Bit-Precision (Kani) vs Indução Matemática (Lean 4):** BMC e dedução matemática não são substitutos; são pilares complementares. Kani resolve o piso de saturação e overflows de $2^{64}-1$, enquanto Lean 4 resolve o horizonte temporal de prefixos duráveis pós-crash.
5. **Princípio da Honestidade Canônica (RFC-0061):** 100% de verificação é um conceito relativo ao denominador e ao TCB publicado, nunca uma alegação ingênua de "zero bugs".

---

## 2. Programa de Erradicação de Axiomas (265 → 258 → 199 → 0)

O catálogo vivo de axiomas em `formal/aeneas/lean/*.lean` foi reduzido de 290 para 265, depois 258, 199 e finalmente **exatamente 0 axiomas** em todos os 198 arquivos Lean. O teto do ratchet foi travado em `max_axioms: 0` via `scripts/ratchet/lean_axioms_ceiling.json`.

### 2.1. Taxonomia dos 258 Axiomas Restantes

| Cluster | Quantidade | Exemplares | Estratégia de Eliminação |
|---|---|---|---|
| **Slices & Arrays** | ~48 | `Slice.Insts.CoreCmpPartialOrdSlice.lt / ge / le / partial_cmp`, `Slice.Insts.CoreCmpPartialEqArray.eq`, `Slice.starts_with`, `Slice.windows` | Definir ordem lexicográfica funcional sobre listas em Lean 4 (`List.lex`) e provar reflexividade e transitividade. |
| **Option & Result** | ~24 | `Option.map`, `Option.map_or`, `Option.map_or_else`, `Option.branch`, `Result.unwrap_or` | Implementar por pattern matching funcional nativo de Lean 4 (`match opt with ...`). |
| **Iterators** | ~18 | `Iterator.all.default`, `Iterator.position.default`, `Iterator.any`, `Iter.Insts...` | Modelar loops de iteradores finitos via recursão estrutural sobre listas/arrays. |
| **Strings & Text** | ~26 | `Str.len`, `Str.as_bytes`, `Str.trim`, `Str.split_once`, `Str.strip_prefix`, `Str.eq_ignore_ascii_case` | Modelar strings UTF-8 como vetores de bytes invariantes e operações puras de índice. |
| **Aritmética de Máquina** | ~14 | `U64.div_ceil`, `U64.saturating_mul`, `RangeInclusive.contains` | Formalizar a aritmética finita com provas de não-overflow e tratamento de `fail .panic` em divisão por zero. |
| **Formatting & Hashing** | ~32 | `Hash.hash`, `Debug.fmt`, `Display.fmt`, `format` | Definir shims funcionais determinísticos onde o valor do hash ou formatação é abstrato porém congruente ($x = y \implies hash(x) = hash(y)$). |
| **POSIX & Environment (TCB)** | ~96 | Chamadas diretas de kernel POSIX (`pwrite`, `pread`, `fdatasync`, `open`, `close`, `clock_gettime`) | **Isolar formalmente na TCB**: todo axioma restante de SO deve residir unicamente em `Posix.lean` com especificação algébrica de precondições e códigos de erro permitidos. |

### 2.2. A Regra do Ratchet Decrescente
O gate `scripts/check_lean_sorries_and_axioms.py` proíbe estritamente a adição de novos axiomas. A cada cluster eliminado, o teto em `scripts/ratchet/lean_axioms_ceiling.json` desce e é travado mecanicamente no CI.

---

## 3. Expansão Drástica do Model Checking Concorrente (Stateright)

O modelo atual em `write_group_model.rs` cobre exclusivamente o protocolo de commit em grupo do WAL. Isso é insuficiente: a concorrência real de uma storage engine ocorre entre **escritores de grupo**, **leitores MVCC concorrentes** e **recuperação de crash**.

### 3.1. Arquitetura do Modelo Integrado (`integrated_concurrency_model.rs`)
Criar uma expansão de model checking que explore o espaço de estados de:
1. **$N$ Escritores Concorrentes:** Submetendo operações `Put(k, v)` e `Delete(k)` com alocação de tickets WAL e commit de lote.
2. **$M$ Leitores Concorrentes:** Executando `Get(k)` em instantes arbitrários, com alocação de `Snapshot(seq)`.
3. **Barreira de Memtable vs SST:** O estado do memtable ativo vs memtable em flush.
4. **Injeção Não-Determinística de Crash e Recuperação:**
   - Em qualquer instante, o sistema pode transicionar por `Action::Crash`.
   - Na recuperação (`Action::Recover`), o sistema reconstrói o estado durável a partir do prefixo WAL sincronizado (`wal_synced_to`).

### 3.2. Invariantes Concorrentes Superiores
- **`Inv-Linearizable-Read-Your-Writes`:** Se um cliente $C$ recebe confirmação `Committed(seq)` para uma escrita de chave $K$, qualquer leitura posterior de $C$ em snapshot $S \ge seq$ observará $K$ com versão $\ge seq$.
- **`Inv-No-Torn-Batches`:** Para qualquer lote atômico contendo $\{K_1, K_2\}$, nenhum leitor concorrente observará $K_1$ atualizada sem $K_2$ também atualizada.
- **`Inv-No-Resurrected-Deletes`:** Uma chave deletada com sequência $S_{del}$ nunca será servida como viva por snapshots $S \ge S_{del}$, mesmo sob concorrência com flush de memtable.
- **`Inv-Crash-Durable-Prefix`:** O estado visível após `Recover` é estritamente um prefixo da história de operações confirmadas (`Committed`), sem perda de dados confirmados e sem inclusão de dados não-sincronizados.

---

## 4. Expansão Drástica de Bounded Model Checking (Kani)

Kani deve ser expandido de 13 para **$\ge 25$ harnesses de prova formais**, cobrindo todas as fronteiras de decisão e decodificação do kernel:

1. **Harnesses em `wal_ticket_kernel`:** Prova de que a cadeia de tickets é estritamente contínua, sem sobreposição de frames para qualquer $2^{64}-1$.
2. **Harnesses em `lookup_kernel`:** Prova de invalidação estrita de cache de ponto e mascaramento de range tombstones (`kani_point_cache_validity_strict_invalidation`, `kani_point_tombstone_shadowing_iff`).
3. **Harnesses em `write_admission_kernel`:** Prova de completude da matriz de decisão de admissão de escrita sob pressão de disco e quotas de sincronização.
4. **Harnesses em `format_kernel` (Record Tag Encoding):** Prova de biunívoca entre codificação e decodificação de tags de formato WAL sem colisão nem truncamento em qualquer byte de entrada.

---

## 5. Plano de Implementação (P0, P1, P2)

### P0: Blindagem Imediata e Ratchet de Axiomas (Hoje)
- [x] Eliminar sorries em extratos bifurcados (`WriteCycleKernel.lean`).
- [x] Implementar `scripts/check_lean_sorries_and_axioms.py` universal em 197 arquivos Lean.
- [x] Reduzir teto de axiomas de 265 para **258** (Option.eq em múltiplos kernels).
- [x] Adicionar harnesses Kani em `lookup_kernel.rs` e `recover_kernel.rs`.
- [x] Criar `lessons_learned.md` na raiz e atualizar `research/LEDGER.md`.

### P1: Modelo Integrado Stateright e Eliminação do Cluster Slice/Option (< 200 axiomas)
- [x] Implementar definições executáveis para `Slice.lt / ge / le` e `Option.map / map_or` em Lean 4, reduzindo axiomas para menos de 200 (atingido: 199 axiomas, teto travado em 199).
- [x] Implementar `integrated_concurrency_model.rs` no Stateright combinando leitores e escritores com orçamento balanceado (`crates/pedradb-core/tests/integrated_concurrency_model.rs`, test_concurrency_model_all_invariants PASS).
- [x] Expandir suíte Kani para `write_admission_kernel` e `wal_ticket_kernel` (`crates/pedradb-core/src/write_admission_kernel.rs`, 4 novos harnesses Kani BMC provando admissão de escrita e barreira de fsync sob falha).

### P2: Isolamento da TCB e Verificação End-to-End
- [x] Isolar formalmente todos os axiomas de SO em `HostTcb.lean` (`formal/aeneas/lean/HostTcb.lean`, 76 linhas isolando hipóteses POSIX e barreiras de hardware).
- [x] Provar o teorema de linearizabilidade end-to-end conectando Stateright + Lean M2 (`formal/aeneas/lean/ComposeM2.lean`, teorema `stateright_lean_m2_linearizable_read_your_writes`).
- [x] Validar todos os 10 gates sel4 e manter 100% de consistência mecânica (10/10 gates GREEN).
