# RFC-0268: Fechamento Definitivo da Auditoria Adversarial Externa — Reprodutibilidade Hermética, Teoremas Universais e Blindagem de CI

- **Status:** Approved for Implementation (P0, P1, P2)
- **Data:** 2026-09-24
- **Autor:** Paulo Cabral Sanz & Antigravity (Pair Programming)
- **Denominador Formal:** Aeneas / Charon / Lean 4.31.0 / Stateright / Rustc 1.97+

---

## 1. Contexto e Motivação: A Crítica Externa

Uma auditoria externa independente sobre a árvore pública (`60d88f4`) analisou a infraestrutura de verificação formal do PedraDB e produziu um relatório rigoroso, pontuando 4 vulnerabilidades metodológicas e de engenharia:

1. **Métricas de Catálogo vs. Cobertura Matemática Real:**
   O catálogo reporta 1.849 funções públicas em 94 arquivos de kernel, das quais 361 estão classificadas e 1.488 estão em *baseline-allowlist*. Essa métrica representa catalogação contábil, e não uma porcentagem matemática de correção comprovada.
2. **Provas Pontuais ("Brinquedo" / Concrete Evaluations):**
   Teoremas como `on_append_grows_written` provam apenas instâncias concretas e restritas (ex.: ledger vazio $\{0, 0, 0\}$ somado a 96 dá 96). Outros teoremas de composição quantificam sobre entradas booleanas abstratas (`may_publish_group wal_io_ok`), sem demonstrar que chamadores concorrentes reais em Rust fornecem os inputs corretos ou preservam a semântica linearizável de leitura (Read-Your-Writes).
3. **Fragilidade e Permissividade de CI:**
   Workflows históricos usavam `set +e` descartando o código de saída de linters e parsers de ratchet permitiam até 107 linhas de `FAIL`. Testes de integração física e compilação real dos teoremas em Lean não eram bloqueantes no pipeline de pull request.
4. **Quebra de Reprodutibilidade Hermética:**
   O `formal/aeneas/lean/lakefile.toml` referenciava um checkout irmão do Aeneas por caminho relativo local na máquina do desenvolvedor (`../../../../aeneas/backends/lean`). Tentativas de reprodução limpa por terceiros falhavam ou estouravam espaço em disco ao clonar a árvore completa do Mathlib.

Este RFC estabelece a solução definitiva para todas as 4 fraquezas, dividida em três fases machine-checked: **P0, P1 e P2**.

---

## 2. Desenho Arquitetural da Solução

```
+-------------------------------------------------------------------------------------------------+
|                                 RFC-0268 TRÍPLICE BLINDAGEM                                     |
+-------------------------------------------------------------------------------------------------+
|                                                                                                 |
|   [P0] REPRODUTIBILIDADE HERMÉTICA & CI FAIL-CLOSED                                             |
|   - lakefile.toml: Git URL + SHA pinado (fallback local transparente)                           |
|   - scripts/reproduce_lean_proofs.sh: clone shallow hermético, sem explosão de Mathlib          |
|   - CI: set -euo pipefail universal; ratchet com teto 0 de falhas e fail-closed em crash       |
|                                                                                                 |
|   [P1] GENERALIZAÇÃO UNIVERSAL DE TEOREMAS (ELIMINAÇÃO DE "TOY EVALS")                         |
|   - WriteAck.lean: on_append_grows_written ∀ (a, s, w) (bytes) com indução aritmética           |
|   - on_barrier_honest_promotes: ∀ (a, s, w), min(s, w) = w para todo estado coerente           |
|   - ComposeConcurrent.lean: Composição end-to-end com tickets atômicos e Linearizabilidade     |
|                                                                                                 |
|   [P2] INTEGRAÇÃO FÍSICA & AUDITORIA DE CATÁLOGO                                                |
|   - CI verification-gates.yml: wal_torn_write_adversarial + concurrency_permutation_proofs      |
|   - Gate de Sorries e Axiomas (0 sorries, 0 axiomas) bloqueante no CI                           |
|   - Atualização completa de lessons_learned.md e research/LEDGER.md                            |
+-------------------------------------------------------------------------------------------------+
```

---

## 3. Fatiamento de Implementação

### 3.1 P0: Reprodutibilidade Hermética & CI Fail-Closed
- [x] **P0.1** Atualizar `formal/aeneas/lean/lakefile.toml` para garantir dependência remota declarada com Git SHA pinado e fallback gracioso.
- [x] **P0.2** Criar `scripts/reproduce_lean_proofs.sh` para auditoria externa com um único comando (`bash scripts/reproduce_lean_proofs.sh`), garantindo compilação limpa sem risco de OOM de disco por Mathlib desnecessário.
- [x] **P0.3** Eliminar qualquer ocorrência de `set +e` ou tolerâncias positivas a erros (`max_fail > 0`) em todos os scripts de CI e ratchets. Qualquer falha deve resultar em `exit 1` imediato.

### 3.2 P1: Teoremas Paramétricos Universais em Lean 4
- [x] **P1.1** Em `formal/aeneas/lean/WriteAck.lean`, substituir provas concretas fixadas em 96 por teoremas universais $\forall (a, s, w : \text{U64}) (bytes : \text{U64})$, provando expansão estrita de `written` e preservação invariante de `acked` e `synced`.
- [x] **P1.2** Em `formal/aeneas/lean/WriteAck.lean`, provar universalmente que barreiras honestas promovem `synced` para qualquer prefixo válido.
- [x] **P1.3** Em `formal/aeneas/lean/ComposeConcurrent.lean`, provar invariantes de ticket allocation e exclusão mútua sobre estados concorrentes arbitrários.

### 3.3 P2: Enforçamento de CI & Atualização dos Ledgers
- [x] **P2.1** Integrar os novos testes de integração adversarial no workflow principal `.github/workflows/verification-gates.yml`:
  - `cargo test -p pedradb-core --test wal_torn_write_adversarial`
  - `cargo test -p pedradb-core --test concurrency_permutation_proofs`
  - `python3 scripts/check_lean_sorries_and_axioms.py` (0 sorries, 0 axiomas)
- [x] **P2.2** Registrar os aprendizados e decisões arquiteturais em `lessons_learned.md` e `research/LEDGER.md`.

---

## 4. Critérios de Aceitação Machine-Checked

1. `python3 scripts/check_lean_sorries_and_axioms.py` retorna `GREEN` com 0 sorries e 0 axiomas em 198 arquivos.
2. `python3 scripts/sel4_gap.py --gate` retorna `GREEN` com 10/10 gates.
3. `cargo test -p pedradb-core --test concurrency_permutation_proofs` passa com 2/2 testes OK.
4. `cargo test -p pedradb-core --test wal_torn_write_adversarial` passa com 2/2 testes OK.
5. Todos os scripts de CI rodam com fail-closed (`set -euo pipefail`).
