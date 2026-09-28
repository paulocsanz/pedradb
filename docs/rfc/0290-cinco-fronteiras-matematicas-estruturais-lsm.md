# RFC-0290: Cinco Fronteiras Matemáticas e Estruturais Finais do LSM PedraDB

- **Data:** 2026-09-28
- **Status:** Aprovado e Registrado para Implementação Imediata
- **Escopo:** Exclusivo PedraDB Pure Storage Engine (Sem Montanha, sem cluster, sem componentes de rede)
- **Restrição de Rigor:** `#![forbid(unsafe_code)]`, Zero-Twin Policy (RFC-0270), Rigor Absoluto (RFC-0273)

---

## 1. Contexto e Motivação

Completando o ciclo de axiomatização das 10 fronteiras matemáticas e estruturais de maior rigor identificadas para o motor de armazenamento puro do PedraDB, esta RFC formaliza os 5 itens finais:

1. **Dualidade de Filtro de Intervalo e Blindspot de Range Tombstones sob Bloom Filters:** Imunização contra a ressurreição acidental de chaves deletadas em níveis inferiores decorrente de bypass ingênuo de SST por Bloom Filter Miss quando a tabela abriga um range tombstone abrangente.
2. **Álgebra de Confluência de Sub-Compactação Paralela:** Prova de bisimulação comutativa demonstrando que a partição de tarefas de compactação inter-nível em $P$ fatias concorrentes gera uma sequência de chaves e metadados estritamente indistinguível da compactação sequencial canônica.
3. **Teto Bounded de Retardo e Escalonamento Fair-Share no Group Commit:** Barreira de admissão ponderada por cota de payload, prevenindo que um escritor anômalo massivo (ex: 200 MiB) sequestre e degrade a cauda p99 de latência de escritores microscópicos concorrentes.
4. **Oráculo de Feedback de Utilização e Estabilidade de Readahead em Scans:** Autômato estocástico de controle de janela com contração reativa baseada na razão de consumo real $\rho = \frac{\text{BytesConsumidos}}{\text{BytesPré-carregados}}$, impedindo a amplificação de leitura e a poluição de cache em scans curtos ou híbridos.
5. **Bisimulação de Camada Transacional Local e Teorema de Transparência Causal (Read-Your-Own-Writes):** Composição monádica determinística do overlay de escrita volátil local com o snapshot do motor, garantindo isolamento absoluto de escritas concorrentes e consistência causal de leituras no escopo da transação.

---

## 2. Fronteira 1: Dualidade de Filtro de Intervalo e Blindspot de Range Tombstones sob Bloom Filters

### 2.1 O Problema
Filtros de Bloom tradicionais respondem se uma chave pontual $k$ pode estar presente no SSTable. Range tombstones $[s, e) @ t$ cobrem intervalos contínuos de chaves. Se um leitor pontual executar `Get(k)` contra um SST em $L_1$ e o filtro de Bloom acusar `BloomMiss(SST, k)`, pular a leitura do SST é catastrófico se a tabela contiver um Range Tombstone cobrindo $k$. O salto ignora o tombstone e ressuscita uma versão obsoleta de $k$ residente em $L_2$ ou níveis inferiores.

### 2.2 Invariante Matemático
A poda segura de um SSTable durante uma busca pontual por chave $k$ é governada pela barreira dual booleana:
$$\text{PruneSST}(SST, k) \iff \text{BloomMiss}(SST, k) \land \neg \text{RangeCover}(SST, k)$$
onde $\text{RangeCover}(SST, k) \equiv \exists [s, e) \in \text{Tombstones}(SST): s \le k < e$.
Se $\text{RangeCover}(SST, k)$ for verdadeiro, a poda pelo Bloom filter é terminantemente desabilitada, forçando a avaliação do tombstone para o cálculo correto da visibilidade da chave.

---

## 3. Fronteira 2: Álgebra de Confluência de Sub-Compactação Paralela

### 3.1 O Problema
Para acelerar compactações volumosas entre níveis $L_i \to L_{i+1}$, o particionamento em $P$ sub-fatias executadas concorrentemente em threads de trabalho não pode violar a monotonicidade das chaves nem fragmentar transações multi-key ou criar gaps temporais na fusão de iteradores do MANIFEST.

### 3.2 Invariante Matemático
Seja $\mathcal{K}$ o espaço de chaves particionado em $P$ intervalos contíguos disjuntos $[k_{p-1}, k_p)$.
Para qualquer conjunto de fatias $\text{Slice}_1, \dots, \text{Slice}_P$:
$$\bigoplus_{p=1}^P \text{SubCompact}(\text{Slice}_p) \equiv \text{SequentialCompact}(\text{FullLevel})$$
com $\forall p \in [1, P-1]: \max(\text{Keys}(\text{Out}_p)) < \min(\text{Keys}(\text{Out}_{p+1}))$, provando ausência de interseção entre arquivos de saída e garantindo que o fecho da versão após o edit seja canonicamente idêntico.

---

## 4. Fronteira 3: Teto Bounded de Retardo e Escalonamento Fair-Share no Group Commit

### 4.1 O Problema
No pipeline de escrita em grupo (*group commit*), se múltiplos escritores chegam concorrentemente, a inclusão indiscriminada de lotes gigantescos no mesmo grupo de sincronização física obriga escritores de baixa latência a aguardar a serialização e `fdatasync` de dezenas de megabytes de dados alheios, explodindo a cauda de latência.

### 4.2 Invariante Matemático
Cada escritor $w_i$ com payload $B(w_i)$ e classe de prioridade é admitido com base em um teto máximo de tamanho por grupo $\mathcal{B}_{\text{group\_max}}$. Se $B(w_i) > \mathcal{B}_{\text{isolated\_threshold}}$, o lote é isolado em um pipeline autônomo, provando o teto de latência limitado:
$$\forall w_i \text{ (pequeno)}: \quad \text{WaitTime}(w_i) \le \Delta_{\text{batch\_drain}} + \Delta_{\text{fdatasync\_bounded}}$$
onde $\Delta_{\text{fdatasync\_bounded}} \le \frac{\mathcal{B}_{\text{group\_max}}}{\text{Throughput}_{\text{disk}}}$, garantindo isolamento temporal estrito.

---

## 5. Fronteira 4: Oráculo de Feedback de Utilização e Estabilidade de Readahead em Scans

### 5.1 O Problema
Iteradores que realizam readahead agressivo com duplicação geométrica da janela ($\mathcal{W} \gets 2\mathcal{W}$) degradam a performance quando o cliente alterna entre varreduras curtas (*point-like short scans*) e buscas aleatórias, provocando amplificação de leitura I/O desnecessária e expulsão de dados quentes da cache de páginas.

### 5.2 Invariante Matemático
O tamanho da janela de pré-carregamento $\mathcal{W}$ é governado por um autômato de feedback de consumo:
$$\rho = \frac{\text{BytesConsumidos}}{\text{BytesPréCarregados}}$$
$$\mathcal{W}_{n+1} = \begin{cases} \min(\mathcal{W}_{\text{max}}, 2 \cdot \mathcal{W}_n) & \text{se } \rho \ge \rho_{\text{expand}} \\ \max(\mathcal{W}_{\text{min}}, \lfloor \mathcal{W}_n / 2 \rfloor) & \text{se } \rho < \rho_{\text{contract}} \\ \mathcal{W}_n & \text{caso contrário} \end{cases}$$
provando que a amplificação de leitura assintótica em scans curtos é delimitada estritamente por $O(1) \cdot \mathcal{W}_{\text{min}}$.

---

## 6. Fronteira 5: Bisimulação de Camada Transacional Local e Teorema de Transparência Causal

### 6.1 O Problema
Em transações com semântica *Read-Your-Own-Writes* (RYOW), o leitor precisa consultar simultaneamente o buffer de escrita local e o snapshot imutável do banco de dados, sem que mutações transacionais em andamento vazem para outras consultas e sem que deleções locais ressuscitem valores do snapshot.

### 6.2 Invariante Matemático
A função de avaliação pontual da visão composta $\text{ReadView} = \text{Overlay}(\text{LocalBatch}) \circ \text{Snapshot}(\mathcal{V})$ satisfaz a regra monádica determinística:
$$\text{Eval}(k, \text{ReadView}) = \begin{cases} \text{Some}(v) & \text{se } \text{Local}(k) = \text{Put}(v) \\ \text{None} & \text{se } \text{Local}(k) = \text{Delete} \\ \text{Snapshot}(k) & \text{se } k \notin \text{Local} \end{cases}$$
Provamos por bisimulação formal que qualquer sequência de leituras intermediárias sob o overlay é isomórfica ao estado resultante de aplicar linearmente todas as mutações no snapshot.

---

## 7. Estratégia de Verificação

1. **Rust Seguro:** Implementado sob `#![forbid(unsafe_code)]`.
2. **Miri Gate:** Verificado sob Tree-Borrows (`scripts/miri_concurrency_gate.sh`).
3. **Continuous Verification Chain:** Integrado como Estágio 25 em `scripts/verify_continuous_chain.sh`.
