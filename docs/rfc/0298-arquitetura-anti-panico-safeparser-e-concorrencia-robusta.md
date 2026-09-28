# RFC-0298: Defesa Arquitetural Contra Pânicos e Parsing I/O Inseguro: SafeCursor, Zero-Panic Invariants e Concorrência Resiliente

- **Status:** Proposto e Aprovado para Implementação
- **Data:** 2026-09-28
- **Autores:** PedraDB Architecture, Reliability & Systems Security Teams
- **Escopo:** Todo o workspace PedraDB (`pedradb-core`, `pedradb-store`, `pedradb-raft`, `pedradb-ops`, `rocksdb-compat`, `tests/`)

---

## 1. Contexto e Motivação

Após a implementação do **RFC-0295** (fechamento de TCB, glue code, linearizabilidade e DST), auditorias aprofundadas focadas em superfícies de deserialização de disco, rede e estruturas de concorrência revelaram uma classe inteira de falhas graves de robustez (*low-hanging fruits* sistêmicos e hazards de execução física):

1. **A Epidemia do `slice[..].try_into().unwrap()`:** Em dezenas de módulos de I/O, decodificadores fatiam buffers de disco e de rede com slices nus, seguidos de conversões `.try_into().unwrap()`. Qualquer truncamento físico, bitrot ou payload hostil desencadeia pânico incondicional (`SIGABRT`), derrubando o processo do banco de dados em vez de retornar `Result::Err`.
2. **Aritmética de Índices Insegura e Overflow de Slices:** Checagens de limites baseadas em adições ordinárias `pos + len > buf.len()` sofrem de estouro de inteiro (*integer overflow wrap*). Quando `len` é próximo a `usize::MAX`, a soma sofre wrap, burlando a checagem, culminando em tentativa de fatiamento fora dos limites da memória e panic fatal.
3. **Exaustão de Heap Pré-Alocação (*Unbounded Vec Allocation DoS*):** Leitores de quadros de rede e arquivos de log confiam cegamente em campos de contagem `n` ou tamanho `len`, executando `Vec::with_capacity(n)` ou `vec![0u8; len]` (até 16–64 MB) antes de validar se o restante do buffer ou conexão possui bytes suficientes, abrindo o servidor a exaustão imediata de RAM (*OOM Kill*).
4. **Envenenamento de Travas (*Mutex Poisoning Cascade*):** O uso de `.lock().unwrap()` em pools assíncronos (`AsyncPoolDecoupling`) transforma qualquer pânico pontual em uma reação em cadeia: a trava é contaminada (*poisoned*) e todos os demais fios do banco de dados entram em colapso.
5. **Vulnerabilidades de Protocolo de Rede:** Servidores TCP (`pedradb-raft`, `pedradb-store`) realizam `thread::spawn` irrestrito por conexão sem timeout de I/O (vulnerabilidade clássica a Slowloris e exaustão de descritores de thread) e ignoram bytes residuais ao término de mensagens (`no-EOF validation`), permitindo injeção e contrabando de comandos (*command smuggling*).

Este RFC define os padrões arquiteturais permanentes e a matriz P0/P1/P2 para extirpar de vez essa classe de bugs do PedraDB.

---

## 2. Matriz de Requisitos e Entregáveis

### Nível P0 — Bloqueadores de Integridade e Eliminação de Pânicos

- **P0.1: Implementação Canônica do `SafeCursor` e `DecodeError` (`pedradb-core`)**
  - Módulo `pedradb_core::codec` expondo `SafeCursor<'a>`, com operações aritméticas sempre protegidas por `checked_add`, fatiamento seguro `read_exact`, decodificadores le/be primitivos (`u8`, `u16`, `u32`, `u64`, `u128`), leitura segura de fatias prefixadas por tamanho (`read_length_prefixed_bytes`) e validação de término estrito (`ensure_fully_consumed`).
  - Banimento do fatiamento nu seguido de `.unwrap()` em decodificadores de dados.

- **P0.2: Blindagem Anti-Pânico nos Decodificadores de Storage (`pedradb-core`)**
  - `bloom_kernel.rs`: Corrigir `decode_partitioned` eliminando overflow de adição `pos + n` e conversão de tamanho.
  - `prefix_delta_restart_kernel.rs`: Proteger o cálculo de `restart_array_bytes`, `with_capacity(num_restarts)` e a checagem com multi-operandos `pos + unshared_len + val_len > restart_start`.
  - `history_kernel.rs`: Proteger `sidecar_may_affect`, `Manifest::decode` e `verify_bloom_sidecar` contra adições vulneráveis a wrap e fatiamentos diretos.

- **P0.3: Blindagem Anti-Pânico e Anti-Smuggling no `pedradb-store`**
  - `lib_kernel.rs`: Corrigir `decode_membership`, `take_bytes`, `decode_u64_list`, `decode_key_list`, `decode_entry`, `decode_log`, `decode_one_log_rec` e `decode_snap`.
  - `msg_kernel.rs`: Corrigir `take_u64`, avançar offset em `RequestVoteReply` e impor `cursor.ensure_fully_consumed()` em todas as mensagens de peer (`PeerMsg::decode`).
  - `tcp_kernel.rs`: Proteger `take_bytes` contra overflow e exigir `ensure_fully_consumed()` em `WireMsg::decode`.

- **P0.4: Blindagem Anti-Pânico e Anti-Crash no `pedradb-raft`**
  - `persist_kernel.rs`: Corrigir `read_bytes`, `read_u32`, `read_u64`, `load_hard_state_on`, `load_commit_on` e `decode_log`.
  - `net_kernel.rs`: Corrigir `take_bytes`, `take_u32`, `take_u64`, `decode_rv_reply2`, `decode_ae_reply` e `decode_ae`.

---

### Nível P1 — Resiliência de Alocação e Desintoxicação de Concorrência

- **P1.1: Invariante de Alocação Bounded por Capacidade Residual**
  - Nenhum container dinâmico pode reservar capacidade `with_capacity(n)` a partir de dados de disco ou rede sem limitação estrita por `cursor.remaining() / min_element_size`.
  - Aplicado em `pedradb-ops` (`read_warch`), `prefix_delta_restart_kernel`, `pedradb-store` e `pedradb-raft`.

- **P1.2: Resiliência a Envenenamento de Travas (*Poison Recovery*)**
  - Em `async_pool_decoupling_kernel.rs`, substituir chamadas `.lock().unwrap()` por recuperação resiliente (`.lock().unwrap_or_else(|p| p.into_inner())`).

- **P1.3: Pipeline Assíncrono Tolerante a Descarte de Tickets**
  - Em `async_pool_decoupling_kernel.rs:acknowledge_committed_batch`, permitir avanço progressivo monotonicamente seguro sem travar a esteira em caso de falhas prematuras ou descarte de tarefas com tickets intermediários.

- **P1.4: Blindagem de I/O Operacional (`pedradb-ops`)**
  - Corrigir `read_warch` e `Catalog::decode` em `pedradb-ops/src/lib.rs` contra pânicos de fatiamento e exaustão de memória.

---

### Nível P2 — Hardening de Protocolo de Rede e Bateria de Verificação

- **P2.1: Mitigação DoS de Conexões de Rede (Timeouts e Limite de Buffer)**
  - Configurar timeouts explícitos de leitura e escrita (`Duration::from_secs(15)`) em conexões aceitas no `pedradb-raft` e `pedradb-store`.
  - Proteger contra pré-alocação maciça de 64 MB / 16 MB em conexões não autenticadas.

- **P2.2: Equidade de Concessão no LockTable (RocksDB Compat)**
  - Mitigar o efeito de manada (*thundering herd*) e inanição no `LockTable` sob contenda pesada de 2PL.

- **P2.3: Bateria de Testes DST e Adversarial Anti-Pânico**
  - Suíte de testes `tests/rfc0298_safecursor_antipanic.rs` injetando buffers truncados, mutados e com overflows aritméticos nos decodificadores de todos os crates, validando a garantia de *zero-panic*.

---

## 3. Especificação Técnica do SafeCursor

```rust
pub struct SafeCursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SafeCursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self;
    pub fn remaining(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn position(&self) -> usize;
    pub fn set_position(&mut self, pos: usize) -> Result<(), DecodeError>;
    pub fn read_u8(&mut self) -> Result<u8, DecodeError>;
    pub fn read_u16_le(&mut self) -> Result<u16, DecodeError>;
    pub fn read_u32_le(&mut self) -> Result<u32, DecodeError>;
    pub fn read_u64_le(&mut self) -> Result<u64, DecodeError>;
    pub fn read_exact(&mut self, len: usize) -> Result<&'a [u8], DecodeError>;
    pub fn read_length_prefixed_bytes(&mut self, max_len: usize) -> Result<&'a [u8], DecodeError>;
    pub fn ensure_fully_consumed(&self) -> Result<(), DecodeError>;
}
```
