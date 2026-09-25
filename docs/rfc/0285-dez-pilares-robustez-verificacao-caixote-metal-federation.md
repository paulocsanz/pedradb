# RFC-0285: Os Dez Pilares de Robustez e Verificação Formal do Ecossistema Caixote (Federation, Metal e Malha Distribuída)

- **Status:** Proposto e Implementado
- **Data:** 2026-09-25
- **Autores:** Equipe de Engenharia e Verificação Formal PedraDB / Caixote
- **Áreas:** `crates/pedradb-core`, `crates/pedradb-fold`, `crates/pedradb-dcs`, `crates/pedradb-store`

---

## 1. Sumário Executivo

O ecossistema Caixote opera sobre nós físicos bare-metal, virtualização leve e redes sobrepostas (WireGuard mesh) conectando serviços federados e clusters PedraDB. Na interseção entre a teoria matemática de verificação e a realidade de infraestrutura (DevOps/SRE), emergem falhas que os modelos simplificados não capturam:
1. Suspensão de máquinas virtuais (VM freeze) e starvation de loops assíncronos no Tokio, invalidando silenciosamente leases de consenso.
2. Fragmentação oculta e blackholes de MTU em túneis WireGuard (MTU 1420 bytes).
3. Ressurreição de chaves zumbis por dessincronização entre snapshots atômicos e streams de deltas CDC no fold federado.
4. Corrupção e arquivos órfãos causados pela omissão do `fsync` em diretórios-pai sob cortes abruptos de energia no metal.
5. Contaminação cruzada de namespaces multi-tenant por colisões de prefixo binário em consultas de range.
6. Starvation e deadlock mútuo quando operações de I/O em disco bloqueiam o event loop responsável pelos heartbeats de rede.
7. Falsos quóruns e split-brain sob partições assimétricas com enlaces unidirecionais.
8. Colapso operacional por exaustão de descritores de arquivo (`EMFILE`) durante rajadas simultâneas de reconexão.
9. Impasses de boot causados por arquivos de trava (`LOCK`) remanescentes de processos mortos por SIGKILL ou reinicialização de nó.
10. Corrupção silenciosa de dados por truncamento de schema durante janelas de rolling upgrade federado.

Este RFC estabelece a base axiomática, implementa os dez kernels formais correspondentes dentro de `pedradb-core` e comprova sua correção via 10 baterias de testes exaustivos com oráculos de anti-vacuidade.

---

## 2. Os Dez Pilares Formais

### Pilar 1: Validade de Leases sob Congelamento de Runtime (`lease_expiration_guard_kernel.rs`)
- **Axioma:** Uma mutação ou leitura linearizável só pode ser despachada se o tempo decorrido desde a concessão do lease mais uma margem de segurança estrita $\tau_{\text{slack}} + 2\epsilon_{\text{drift}}$ for inferior à duração do lease:
  $$t_{\text{now}} - t_{\text{granted}} + \tau_{\text{slack}} + 2\epsilon_{\text{drift}} < \Delta_{\text{duration}}$$
- **Garantia:** Se a VM for pausada ou o runtime sofrer starvation, ao acordar o lease é sumariamente invalidado em modo *fail-closed*, impedindo leituras stale ou escritas de líderes defasados.

### Pilar 2: Fragmentação Atômica e Barreira de MTU WireGuard (`mesh_mtu_fragmentation_kernel.rs`)
- **Axioma:** Nenhuma mensagem física excede o MTU da malha (1420B menos cabeçalhos IP/UDP/WireGuard). Mensagens maiores são divididas em fragmentos atômicos monotônicos com CRC32C composto. O commit na máquina de estados só é elegível após a recepção estrita de $100\%$ dos fragmentos consecutivos.
- **Garantia:** Eliminação categórica de blackholes de PMTU onde pacotes com a flag DF (*Don't Fragment*) são descartados silenciosamente pelo kernel Linux.

### Pilar 3: Bisimulação e Continuidade Estrita de Cursores Federados (`federated_cursor_continuity_kernel.rs`)
- **Axioma:** A transição do snapshot atômico para o stream de deltas exige continuidade estrita de sequência ($S_{\text{delta}} == S_{\text{snapshot}} + 1$). Qualquer gap dispara interrupção fail-closed, e qualquer evento retroativo ($\le S_{\text{snapshot}}$) é descartado de forma comutativa e idempotente.
- **Garantia:** Prova de bisimulação: o estado derivado de um snapshot seguido de deltas é rigorosamente idêntico ao estado de um dump completo no instante final, erradicando ressurreição de chaves deletadas.

### Pilar 4: Ordem POSIX de Diretório-Pai sob Queda de Energia (`metal_fsync_barrier_kernel.rs`)
- **Axioma:** A criação e substituição atômica de arquivos exige a barreira completa:
  $$\text{write} \prec \text{fdatasync}(\text{tmp}) \prec \text{fsync}(\text{parent\_dir}) \prec \text{rename}(\text{tmp}, \text{dest}) \prec \text{fsync}(\text{parent\_dir})$$
- **Garantia:** Em caso de perda repentina de energia no hardware bare-metal, os metadados do diretório-pai refletem com exatidão ou a versão antiga íntegra ou a nova versão commitada, sem gerar arquivos zerados ou órfãos.

### Pilar 5: Não-Interferência e Injetividade de Prefixos Multi-Tenant (`multitenant_prefix_isolation_kernel.rs`)
- **Axioma:** A função de codificação de chaves compostas é bijetora e prefix-free com terminação delimitada por tamanho (length-prefixed). Para quaisquer tenants distintos $T_A \neq T_B$ e quaisquer chaves $K_1, K_2$:
  $$\text{PrefixFreeKey}(T_A, K_1) \not\sqsubseteq \text{PrefixFreeKey}(T_B, K_2)$$
- **Garantia:** Impossibilidade matemática de uma busca por prefixo de um cliente vazar chaves de outro cliente, mesmo na presença de bytes nulos (`0x00`) e caracteres especiais.

### Pilar 6: Desacoplamento Assíncrono do Group Commit (`async_pool_decoupling_kernel.rs`)
- **Axioma:** O escalonamento da fila de escrita em grupo (Group Commit) utiliza passagem lock-free de tickets em canal desacoplado, garantindo que I/O bloqueante de fsync/pwrite execute em threads dedicadas e nunca monopolize as worker threads do runtime assíncrono (Tokio).
- **Garantia:** Ausência de inanição ($K$-bounded overtaking) e garantia de que os heartbeats do cluster e da malha continuem sendo despachados sem atraso sob saturação pesada de escrita.

### Pilar 7: Rejeição de Quórum Assimétrico e Confluência Bipartida (`asymmetric_partition_quorum_kernel.rs`)
- **Axioma:** Uma confirmação de quórum entre os nós $i$ e $j$ é válida se e somente se o canal de comunicação for bidirecional ($\text{CanSend}(i \to j) \land \text{CanSend}(j \to i)$). Enlaces assimétricos (half-open) são descartados na formação da maioria.
- **Garantia:** Prevenção categórica de split-brain em partições direcionadas de rede causadas por regras incorretas de firewall ou descarte unidirecional de pacotes TCP.

### Pilar 8: Governança Estrita de Descritores de Arquivo (`fd_quota_governor_kernel.rs`)
- **Axioma:** A alocação total de FDs é limitada estaticamente a $\text{FD}_{\text{storage}} + \text{FD}_{\text{mesh}} \le \text{MaxCap} < \text{OS\_Limit}$. Quando a cota de conexões da malha atinge o limiar, novas conexões sofrem backpressure ordenado sem pânico.
- **Garantia:** Imunidade a `EMFILE / ENOSPC` durante tempestades de reconexão de nós federados.

### Pilar 9: Idempotência de Trava com Linux Boot-ID (`boot_id_lockfile_kernel.rs`)
- **Axioma:** O arquivo `LOCK` contém uma tupla `(boot_id, pid, timestamp)`. Uma trava é considerada viva se e somente se o `boot_id` coincidir com o kernel atual e o processo existir com o mesmo token de vida. Caso contrário, a trava é considerada zumbi e expurgada atomicamente.
- **Garantia:** Reinicializações após SIGKILL ou reinício do servidor de metal nunca exigem remoção manual do arquivo de trava pelo operador.

### Pilar 10: Homomorfismo Bidirecional de Schemas em Rolling Upgrade (`rolling_upgrade_homomorphism_kernel.rs`)
- **Axioma:** Um decodificador da versão $V_1$ que recebe uma mensagem da versão $V_2$ ($V_2 > V_1$) preserva os campos de extensão desconhecidos em um envelope opaco comutativo, de tal forma que re-serializar a mensagem a devolve intacta à malha:
  $$\mathcal{E}_{V_2}(\mathcal{D}_{V_1}(\text{payload}_{V_2})) \equiv \text{payload}_{V_2}$$
- **Garantia:** Tolerância estrita e ausência de perda de metadados em ambientes federados onde diferentes nós rodam versões adjacentes de software simultaneamente.

---

## 3. Conformidade e Validação

Todos os 10 pilares são implementados como funções puras `#![forbid(unsafe_code)]` em `crates/pedradb-core`, com suítes de teste dedicadas contendo oráculos falsificáveis contra mutantes degenerados ("as-is / dente").
