# RFC-0189 P1.1 — desenho: frame do grupo em buffer próprio + ticket de ordem

**Data:** 2026-09-10
**Status:** design aprovado para implementação em P1.2/P1.3 (fatia de desenho)

## Fato motivador (código, hoje)

O líder do pipeline (`concurrent.rs`, `lead_pipeline_group` ~L1414-1448) faz, DENTRO de
`wal.lock()`:

1. `w.encode_write_op_batches(&[group_ops])` — fragmenta os ops **no frame interno do
   `WalWriter`** (`fragment_encoded_len` escreve no buffer reusável do writer);
2. `w.write_pending_frame()` — `write_all` (syscall) + avança o cursor de append.

P0.1 atribuiu 0,61–0,98 µs/op ao syscall `wr` — todo esse tempo está na seção crítica
que os líderes serializam. O caminho verificado/G1 (~L2048) tem o mesmo
`write_pending_frame` (+ `sync_data` quando `need_sync`) sob o mesmo lock.

Dois fatos tornam o corte barato:

- **O encoder já é desacoplável**: `WalWriter::fragment_encoded_len(ops, buf)` fragmenta
  em QUALQUER `&mut Vec<u8>` do chamador — só a posse do buffer prende o encode ao lock.
- **O EnvFile já tem o padrão posicional**: `read_exact_at` (default portátil
  seek/read/seek-back + override Unix `FileExt`) — escrever é o simétrico.

## Desenho

### 1. Buffer do grupo (fora do lock)

O líder codifica para um scratch `Vec<u8>` próprio (TLS do líder — waiters nunca
codificam; `share_consecutive_equal_values` continua antes, inalterado). API nova no
`Wal`: `encode_group_frame(&ops, &mut out) -> u64` — mesma fragmentação
(`fragment_from`), mesmos header+CRC, bytes idênticos aos de
`encode_write_op_batches`+`write_pending_frame`.

### 2. Ticket de ordem (lock curto, sem encode, sem syscall)

Sob `wal.lock()` apenas: `let ticket = w.reserve_frame(len)` — devolve
`base = position()`, avança o cursor de reserva, cuida do `reserve_space`/prealloc.
O `Wal` passa a ter DOIS cursores:

- `reserved_to` — fronteira de alocação (avança no reserve, sob o lock);
- `position()` — fronteira escrita (avança quando o pwrite retorna; usado pelo ledger
  verificado e por sync/close).

A ordem dos bytes NO ARQUIVO = ordem dos tickets, independente da ordem de conclusão
dos syscalls. `write_all_at(frame, ticket)` fora do lock.

### 3. EnvFile::write_all_at (nova seam)

`fn write_all_at(&mut self, buf: &[u8], at: u64) -> io::Result<()>` — posix =
`pwrite` stateless (paralelo de verdade); default portátil = seek/write/seek-back
(cursor salvo — correto, não paralelo; serve os Envs de teste/sim). Capability
`positional_writes()` no open: sem ela o `Wal` fica no caminho de hoje
(`write_pending_frame` in-lock) — **fallback ativo**, requisito do P1.3.

### 4. Drenagem (close/sync)

`inflight_writes: AtomicUsize` no `Wal` (reserve→write). `Db::sync`/close esperam
`inflight == 0` antes do barrier/fsync — sem isso um `position()` de leitor pode ver
bytes ainda não escritos.

### 5. Crash/byte-identity

- Arquivo final: offsets dos tickets = offsets de append sequencial por construção ⇒
  **byte-idêntico** para a mesma sequência de ops (gate: `cmp wal-before/after.bin`).
- Crash com write em voo: o intervalo não escrito vira um buraco de zeros; a
  recuperação para no buraco (header/CRC) = PointInTime no prefixo — o MESMO contrato
  de cauda rasgada de hoje; grupos além do buraco descartam-se inteiros (atomicidade
  por grupo preservada — "all records land or none do").
- G1/verificado (`need_sync`): `fdatasync` DEPOIS do pwrite retornar, off-lock (fd
  compartilhado; barrier é do arquivo inteiro, ordem irrelevante); Fence em falha
  inalterado.

### 6. P1.3 (io_uring) sobre o mesmo ticket

SQEs encadeados por ticket (`IOSQE_IO_LINK`/drain), um submitter; completude ordenada
⇒ mesmos bytes; fallback = pwrite P1.2.

## Gates de implementação (P1.2)

- `off_lock_write_order_survives_two_leaders` — dois líderes, reserve/write
  intercalados; bytes no arquivo na ordem de encode (ticket).
- `cmp` wal-before/after.bin idêntico (captura ANTES de tocar qualquer linha).
- Suíte de recuperação crash/reopen/torn verde (ex. `async_ok_write_wal_without_fsync_survives_reopen`,
  `rfc0185_pipeline_group_single_record_recovers`).

## Riscos

- Drenagem esquecida em close/sync (mitigada pelo join `inflight == 0`).
- Prealloc na fronteira de chunk dentro do reserve (mesmo lock de hoje — sem custo novo).
- Envs de teste com default portátil: cursor compartilhado ⇒ o default precisa &mut self
  (o `Wal` já é atrás de Mutex; o caminho default volta a serializar — aceitável, é o
  fallback).
