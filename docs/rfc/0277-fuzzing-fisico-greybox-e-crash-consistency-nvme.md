# RFC-0277: Fuzzing de Parsers Físicos (Greybox com Sanitizers) e Crash-Consistency de Blocos NVMe (Power-Fail Hardware-Accurate)

- **Status:** Approved & Implemented
- **Data:** 2026-09-24
- **Autor:** PedraDB Verification & Core Architecture Team
- **Contexto:** Superação dos limites além do TCB estrito do código-fonte — garantia contra corrupção física de mídia (bitrot de SSD/NVMe) e reordenação de blocos voláteis em corte repentino de energia.

---

## 1. Motivação e Fronteiras de Confiabilidade

Até o RFC-0276, o PedraDB atingiu 100% de verificação em nível de AST, Lean 4 ($m_2 = 100\%$), DST em disco POSIX e fuzzing de mutação de código.

No entanto, dois vetores de falha física do mundo real operam **abaixo** da camada de abstração do sistema operacional:
1. **Corrupção Fisiológica de Dados (Bitrot / Malicious Payloads):**
   - O disco entrega bytes arbitrários corrompidos (ECC não corrigido, radiação de partículas alfa, corrupção de controladora, arquivo truncado em transferência).
   - O parser do banco não pode entrar em pânico (`panic!`), nem entrar em loop infinito, nem alocar memória descontrolada (`OOM`). Deve falhar fechado com `Err(...)` de forma determinística e segura.
2. **Reordenação Física de Blocos NVMe em Falha de Energia (Power-Cut):**
   - Controladoras de SSDs modernos possuem DRAM volátil para reordenação de escritas de blocos (LBA remapping).
   - Entre duas barreiras físicas (`fdatasync`), setores de 512B ou blocos de 4096B podem atingir as células NAND fora de ordem temporal, ou serem interrompidos na metade da escrita de um bloco (*torn sector write*).
   - O protocolo de WAL e SST do PedraDB deve garantir que, para **toda permutação de blocos voláteis sobreviventes a um corte de energia**:
     - O banco reabre com sucesso (`Db::open`).
     - A propriedade **D1** (durabilidade de writes confirmados) é preservada intacta.
     - Nenhum dado fantasma (resurrected/corrupted keys) é injetado.

---

## 2. Pilar I: Fuzzing Greybox Guiado por Cobertura nos Parsers Físicos

Implementação de um motor de fuzzing físico diferencial que bombardeia sistematicamente com mutações em nível de bit:
- **Alvo 1: SST Data & Index Blocks (`table_kernel.rs`, `table.rs`):**
  - Mutações de footers, magic numbers, tamanhos de blocos, contadores de restarts e offsets de índice.
- **Alvo 2: Framing de Fragmentos WAL (`wal_recover_kernel.rs`, `wal.rs`):**
  - Fragmentos isolados, headers de tamanho adulterados, tipos de registro inválidos (fora do enum de 1 a 4), CRC falso e comprimentos descompassados.
- **Alvo 3: Headers de Filtro Bloom (`bloom_kernel.rs`):**
  - Metadados corrompidos de bits/chaves, contadores de hash $k > 30$, arrays de bits truncados.

**Oráculo Invariante:**
- Zero panics (`catch_unwind` em todas as rotas).
- Tempo de resposta limitado ($< 10\text{ms}$ por payload, detecção de loops).
- Falha limpa em `Result::Err` para qualquer anomalia.

---

## 3. Pilar II: Motor de Crash-Consistency de Blocos NVMe (Power-Fail Replay)

Implementação de um emulador de dispositivo de blocos NVMe (`BlockDeviceCrashEmulator`):
- O emulador intercepta todas as chamadas de `pwrite` e mapeia para blocos de 4 KiB (setores de 512B).
- Mantém um *Volatile In-Flight Cache* simulando a DRAM da controladora NVMe.
- Chamadas de `fdatasync` drenam o cache volátil para o meio persistente estável.
- Durante um corte de energia (`power_cut`):
  - Blocos voláteis em trânsito são submetidos a permutações de queda: alguns caem, alguns sobrevivem fora de ordem, e blocos limítrofes sofrem *torn write* (gravação parcial de setores).
  - O banco de dados tenta recuperar (`Db::open`) sobre o snapshot resultante.
  - Testa-se a integridade transacional de leituras e a capacidade do banco de continuar aceitando novas escritas.

---

## 4. Integração Contínua (CVC)

Os dois novos motores são adicionados como Estágios 8 e 9 do `verify_continuous_chain.sh`, tornando o pipeline de validação do PedraDB o mais completo e profundo do ecossistema de bancos de dados open-source.
