# RFC-0267 — Verificação de Próxima Geração: Semântica Concreta em Lean 4, Concorrência Real com Loom e Injeção de Falhas Físicas de Disco (Torn-Writes)

**Estado:** ativo (P0, P1, P2 em implementação)  
**Data:** 2026-09-24  
**Parents:** [0261](0261-rumo-a-zero-axiomas-expansao-stateright-kani.md), [0260](0260-endgoals-v4-verificacao-pos-adversarial.md), [0259](0259-auditoria-adversarial-remediacao-verificacao.md), [0061](0061-residuals-sel4-ironfleet.md), [0191](0191-produto-sem-desculpas.md)  
**Alvo:** Superação definitiva de todas as 6 fraquezas identificadas na auditoria pós-0-axiomas, estabelecendo o padrão ouro em verificação formal e testes de resiliência física de databases.

---

## 1. Motivação e O Diagnóstico Adversarial

A erradicação de 100% dos axiomas em Lean 4 (RFC-0261) garantiu a consistência estritamente dedutiva do código extraído. Entretanto, uma análise adversarial rigorosa revelou 6 fraquezas estruturais que separam um modelo formal de laboratório da execução física real de um banco de dados:

1. **A Falácia do Mock Otimista em Lean 4:** Definições reflexivas de stdlib simplificadas (`strip_prefix` sempre retornando `Some`, fatiamento ignorando limites de índice) provam teoremas em mundos onde erros de fatiamento não existem.
2. **O Twin Model Gap no Stateright:** O modelo do Stateright explora uma máquina de estados abstrata escrita à mão em testes, mas não roda nem intercala as primitivas de concorrência reais (`wal/writer.rs`, `AtomicU64`, canais MPSC).
3. **A Lacuna de Falhas Físicas de Hardware (Torn-Writes):** O simulador determinístico (`World`) simula truncamentos limpos de registros inteiros, mas não testa a realidade física de setores de 512 bytes / 4KB corrompidos no meio de um bloco por corte abrupto de energia elétrica.
4. **A Lacuna da Superfície de Execução (42.5k LOC):** 130 arquivos de infraestrutura (handlers, threads de SO, dispatcher) operavam sem uma máquina de estados formalizada.
5. **Limitação de Profundidade no Kani (Bounded Unwind):** Laços de merge e busca analisados apenas até profundidades rasas (2-3 passos).
6. **Fragilidade de Memória Fraca (ARM64 Weak Memory):** Risco de reordenações físicas de CPU (`Acquire`/`Release`) passarem despercebidas em testes monothread no Mac/x86.

O RFC-0267 ataca e elimina cada uma dessas 6 frentes com código de produção provado e testes mecânicos no CI.

---

## 2. Programa de Implementação (P0, P1, P2)

### P0.1: Semântica Concreta em Lean 4 (Fim dos Stubs Otimistas)
- Substituir stubs de `PathKernel.lean` e `FailClosedKernel.lean` por definições que computam comprimento e limites reais:
  - `RangeTo.index`: se `idx > s.len`, falha com `.panic` (idêntico ao Rust); caso contrário, retorna a subfatia exata.
  - `RangeFrom.index`: se `start > s.len`, falha com `.panic`; caso contrário, retorna a subfatia exata.
  - `strip_prefix`: checa casamento estrito de bytes; se divergir, retorna `ok none`.
- Manter o teto travado em **0 axiomas** e **0 sorries** no gate universal.

### P0.2: Concorrência Exaustiva de Código Real com Loom
- Implementar suite Loom (`crates/pedradb-core/tests/loom_concurrency_proofs.rs`) que testa o coordenador real de tickets e publicação de sequências MVCC.
- Validar ausência de data races e preservação estrita de barreiras de memória sob todas as intercalações de threads geradas pelo Loom.

### P1.1: Simulador Físico de Torn-Writes e Injeção de Falhas de Setor
- Criar `torn_write_injector.rs` e suite adversarial de integridade de disco (`wal_torn_write_adversarial.rs`).
- Simular corte de energia em qualquer byte do bloco de 4KB (inclusive no meio de tags de registro e CRCs).
- Provar que `recover_kernel.rs` e `reopen_kernel.rs` identificam corrupção física fail-closed, recuperam o prefixo limpo e nunca sofrem corrupção silenciosa.

### P1.2: Formalização da Máquina de Estados do Dispatcher
- Criar `dispatcher_kernel.rs` com transições de estado estritas para o runtime (`Idle -> Ingesting -> Batching -> Syncing -> Flushing -> Compacting`).
- Garantir que o runtime nunca execute I/O fora de uma transição formalmente permitida.

### P2: Kani Indutivo & Verificação Paramétrica
- Desenvolver harnesses BMC indutivos para operadores de ordenação e decodificação de registros variáveis.

---

## 3. Critérios Irrevogáveis de Aceitação

1. `python3 scripts/check_lean_sorries_and_axioms.py` verde com **0 sorries e 0 axiomas** sob semântica concreta.
2. Suite Loom ou intercalação concorrente executando e provando ausência de races.
3. Teste adversarial de torn writes provando recuperação atômica sem vazamento de dados corrompidos.
4. Todos os 10 gates seL4 verdes (`scripts/sel4_gap.py --gate`).
5. Ledgers e memórias de engenharia consolidados.
