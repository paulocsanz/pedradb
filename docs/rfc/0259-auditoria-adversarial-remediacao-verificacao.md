# RFC-0259 — Auditoria Adversarial Profunda e Remediação da Verificação Formal

**Estado:** ativo (P0/P1 em implementação)  
**Paga:** Rigor e Soundness dos Três Pilares da Verificação (EG2 / EG3)  
**Parents:** [0258](0258-arquitetura-tri-pilar-verificacao-formal-100.md), [0222](0222-escada-ate-o-par-sel4-gap-por-eixo-com-denominador.md), [0061](0061-residuals-sel4-ironfleet.md)

---

## 1. Contexto e Motivação: O Teste do Peer Review Adversarial

A alegação de atingir "100% de verificação formal" sob a ótica dos denominadores sintéticos internos (A2b fns = 99.3%, recuperação A4 = 12/12, gates = 9/9) mascara vulnerabilidades estruturais fundamentais que seriam impiedosamente apontadas por qualquer comitê de conferência de primeira linha (SOSP, OSDI, POPL, PLDI, CAV).

Esta RFC documenta o resultado da **auditoria adversarial profunda** realizada sobre o sistema formal do PedraDB e estabelece o programa mecânico de remediação para blindar a base de verificação contra falsas alegações, vacuidade e suposições não declaradas.

---

## 2. As 9 Fragilidades Estruturais Identificadas

1. **V1 — Três `sorry`s Ativos em `WriteCycleKernel.lean`:**
   Linhas 1746, 1755 e 2915 continham `sorry` em `WriteCut.as_str`, `fmt` e `render`. O script `scripts/lean_extracts.sh` inspecionava apenas `${lib}.lean` e ignorava `${lib}Kernel.lean`, permitindo a passagem de sorries não documentados no TCB.
2. **V2 — 324 Axiomas Não Modelados no Lean 4:**
   Primitivas do stdlib do Rust (`format`, `sort_by`, `hash`, `div_ceil`, `as_slice`) são postuladas como axiomas em 39 arquivos `.lean`, sem modelos denotacionais. Contradições entre axiomas acarretam vacuidade ($False \implies Q$).
3. **V3 — Gap de Superfície de 42.18% (126.548 LOC de Handlers/Glue):**
   Apenas 57.82% das LOC (173.491 de 300.039) pertencem aos kernels Aeneas. Os outros 42.18% (sockets, syscalls, event loop epoll, mimalloc) estão fora do escopo dedutivo.
4. **V4 — Composição M2 Ilusória (88.22% Átomos Desconectados):**
   `ComposeM2.lean` é essencialmente um índice de nomes de strings provados com `:= rfl`. Apenas 39 de 331 átomos participam de lemas de composição reais.
5. **V5 — Horizontes Rígidos no Kani BMC:**
   Apenas 12 harnesses Kani no repositório. O teste de Bloom filter é restrito a chaves de 2–3 bytes e payload $\le 8$ bytes por explosão de estados no CBMC.
6. **V6 — Abstração Finita do Stateright:**
   O modelo de `WriteGroup` restringe-se a 3 clientes e 6 passos, operando sobre um modelo em Rust em vez da engine concorrente de produção.
7. **V7 — Abismo Semântico AST vs Binário:**
   Charon/Aeneas extraem de AST funcional; rustc, LLVM e linker estão no TCB sem validação de tradução de binário.
8. **V8 — Suposição de Consistência Sequencial vs Memória Fraca:**
   Nem Lean nem Stateright modelam `Acquire/Release` e `Relaxed` reorderings do ARM64 / Graviton.
9. **V9 — Barreira de I/O Físico ($fdatasync \ne$ Silício Não-Volátil):**
   Filesystems (`barrier=0`) e discos NVMe com write-cache volátil sem PLP invalidam a hipótese de persistência atômica.

---

## 3. Programa Mecânico de Remediação

### P0 — Blindagem dos Gates de `sorry` e Erradicação em `WriteCycleKernel.lean`
- Modificar `formal/aeneas/lean/WriteCycleKernel.lean` para substituir os 3 `sorry`s por casamento funcional exhaustivo sobre o enum `WriteCut` e implementação sem pânico de strings.
- Atualizar `scripts/lean_extracts.sh` para verificar recursivamente `${lib}Kernel.lean` e barrar qualquer `sorry` na árvore de extrações.

### P1 — Classificação e Redução do Catálogo de Axiomas Lean
- Auditar os 324 axiomas em `scripts/ratchet/lean_axioms_catalog.tsv`.
- Para funções puras aritméticas (`div_ceil`, `saturating_add`, `saturating_mul`), substituir `axiom` por definições executáveis equivalentes no Lean 4 Std.
- Para funções com efeitos ou complexidade (formatação, hash), declarar formalmente no TCB a suposição de pureza e terminação.

### P2 — Ampliação da Composição $m_2$ Além de Nomes Reflexivos
- Transformar `ComposeM2.lean` progressivamente em lemas de implicação de transição de estado, conectando `wal_commit_plan` $\implies$ `reopen_outcome` $\implies$ `get_live`.

### P3 — Declaração Explícita de Fronteiras no TCB
- Atualizar `docs/verification-tcb.md` e `docs/verification-ledger.md` explicitando que o "100%" refere-se à cobertura da álgebra dos kernels cadastrados e não ao binário LLVM nem ao hardware físico.
