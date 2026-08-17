# PedraDB × RocksDB — mapa de garantias (foco: falha de fsync com sync ativo)

**Status:** relatório de comparação, fontes primárias verificadas em 2026-08-17
**Escopo:** `pedradb-core` + Montanha-Store vs RocksDB (branch `main` do upstream no dia da verificação)
**Método:** claims do RocksDB citados por arquivo/fonte do upstream; claims do PedraDB citados por arquivo/linha deste repositório. Comportamentos de terceiros (Postgres/SQLite/etcd/LevelDB) marcados como conhecimento estabelecido — **não** reverificados nesta sessão.

---

## 1. Como cada DB lida com falha de fsync obrigatório (sync: true)

| Sistema | O que acontece após fsync do WAL falhar | Continua aceitando writes? | Recuperação |
|---|---|---|---|
| **PedraDB** (`sync: true` default) | Write retorna `Err`; **handle inteiro fica durability-fenced** (`CoreError::DurabilityFenced`). Memtable **não** aplica o registro; sequência é revertida pelo caller. Errado presumir "registro ausente" — resultado é **incerto** (append pode ter aterrissado). `crates/pedradb-core/src/db.rs:4117-4124`, `error.rs:70-77`, group path `db.rs:4195-4203` | **Nunca** — todo caminho de escrita passa por `ensure_not_fenced()`. Não existe flag que desligue o fence | `close()` + `open()` (rebuild da mem pelo WAL). Sem resume in-process |
| **RocksDB** (`WriteOptions::sync=true`, config default) | Write retorna erro; `WALIOStatusCheck` → `SetBGError(kWriteCallback, wal_related=true)` quando `paranoid_checks` (default true). Severidade: `kWriteCallback + kIOError + paranoid → kFatalError` (`DefaultErrorSeverityMap`). `severity ≥ kHardError → is_db_stopped_=true`; fatal desativa auto-recovery. `db/db_impl/db_impl_write.cc` (`WALIOStatusCheck`, `PreprocessWrite` gate), `db/error_handler.cc` (mapas de severidade) | **Não**, com `paranoid_checks=true`: `PreprocessWrite` rejeita writes seguintes com o bg_error. **Sim**, com `paranoid_checks=false`: erro retorna e o DB segue aceitando writes | `DB::Resume()` manual ("may or may not succeed" — db.h); fatal não auto-resume. No caminho `manual_wal_flush + wal_related`, upstream explicitamente usa kFatalError **pelo mesmo motivo** do nosso fence: WAL/memtable divergentes (comentário em `error_handler.cc`) |
| **RocksDB** ENOSPC específico | `kNoSpace → kHardError` + `SstFileManager` auto-recovery (pode liberar espaço e auto-resumir) | Para, mas com caminho de auto-recuperação transient | Auto-resume ou `Resume()` sem reopen |
| PostgreSQL | Falha de write/flush do WAL → **PANIC** (backend crasha; fail-stop, não continua) *(conhecimento estabelecido)* | Não | Restart do processo / replay do WAL |
| etcd (boltdb) | Falha de fsync → **panic** do processo *(conhecimento estabelecido)* | Não | Restart do membro |
| SQLite | `SQLITE_IOERR` e a transação aborta; journal/WAL protegem contra apply parcial *(conhecimento estabelecido)* | O handle continua; a TX específica falha | Retry da TX |
| LevelDB | Erro retornado; **sem fence** do handle *(conhecimento estabelecido)* | Sim | N/A — risco de continuação sobre meio de armazenamento falho |

**Leitura honesta:** no pedaço "fsync falhou, e daí?", RocksDB com config default e PedraDB têm **a mesma filosofia fail-stop** — ambos param os writes. Diferenças reais:

1. **PedraDB é incondicional**; no RocksDB o fence depende de `paranoid_checks` (que também governa outras coisas) e existe a porta `paranoid_checks=false` onde writes continuam após erro de fsync.
2. **PedraDB tipifica o estado**: `DurabilityFenced` é um erro próprio com contrato de "resultado incerto" documentado no rustdoc; no RocksDB o caller vê o `Status` original e depois o mesmo bg_error em writes seguintes — a semântica de incerteza vive no wiki, não no tipo.
3. **RocksDB tem recuperação mais fina**: ENOSPC vira kHardError auto-recuperável e erros IO "file-scope/retryable" rolam para um **novo** arquivo WAL e reescrevem (auto-resume). PedraDB cerca sempre e exige reopen — mais simples e estrito, menos disponível em falhas transitórias. Trade-off declarado, não deficiência escondida.

---

## 2. Garantia a garantia — onde somos iguais, melhores, piores

### 2.1 Iguais (paridade de contrato)

| Garantia | RocksDB | PedraDB |
|---|---|---|
| `sync=true` → `Ok` implica WAL fsyncado antes do retorno | Sim | Sim (default **ligado**) |
| TX/batch multi-key = **um** registro atômico no WAL | WriteBatch (grupos merged num único registro — `db_impl_write.cc`) | `WriteRecord` único (TX inteira = 1 registro) |
| Publicação crash-safe de SST/MANIFEST (tmp → fsync → rename → sync_dir → MANIFEST/CURRENT atômico) | Sim (+ quarentena de files em estado ambíguo pós-erro de MANIFEST — `error_handler.h`) | Sim (+ GC de SST órfão no open; erros de `sync_dir` propagados) |
| Cauda do WAL truncada por crash → descartada limpo, sem TX parcial | Sim (`kTolerateCorruptedTailRecords`/default) | Sim (torn tail → fim limpo; `wal/reader.rs`) |
| Exclusividade de processo (LOCK) | Sim (db.h: depende de `env->LockFile()`) | Sim (PID LOCK; same-PID re-open rouba) |
| Checkpoint / backup com verify | Sim | Sim (+ PITR por seq, ship-wal, verify — paridade funcional) |
| Tailing do log para replicação | `GetUpdatesSince` | CHANGELOG + `changes(from,to]` + seq pin |

### 2.2 Onde o PedraDB é mais forte (default ou tipo de garantia)

| # | Garantia | PedraDB | RocksDB |
|---|---|---|---|
| 1 | **Durabilidade out-of-the-box** | `OpenOptions::sync` default **true** (`db.rs` `Default`). "Conectou, está durável" | `WriteOptions::sync` default **false** (`options.h`: "Default: false"); o próprio db.h diz "consider setting options.sync = true" em Put/Delete |
| 2 | **Fence de durabilidade incondicional e tipificado** | Sempre cerca; erro próprio documenta "resultado incerto"; sem flag para continuar após fsync falho | Fence depende de `paranoid_checks`; com `false`, writes continuam sobre o mesmo meio falhando |
| 3 | **Fail-closed em corrupção de WAL** | CRC mismatch no meio do log → erro no open (fail-stop). Sem modo "salve o que der" | Default `kPointInTimeRecovery` **para o replay no ponto da corrupção e segue** (disponibilidade sobre fail-stop); há modo `kSkipAnyCorruptedRecords` (salvage) e, com `paranoid_checks=false`, abre com files corrompidos |
| 4 | **Escalonamento por histórico de corrupção** (CORRUPTLOG, RFC-0038 — ⚠️ ainda no working tree, não commitado) | 3º evento fail-stop registrado recusa open **em qualquer modo**: força evacuação em mídia morrendo. Um evento isolado não distingue bitflip de hardware moribundo — só história distingue | Não existe equivalente |
| 5 | **CAS first-class no kernel** | `put_if_absent` / `put_if_eq` / `compare_and_swap` atômicos, fail-closed (`CasMismatch`) | Sem primitiva CAS no `DB` público; exige TransactionDB (`GetForUpdate`) ou Merge |
| 6 | **GC de história com fail-closed explícito** | Watermark + `SnapshotTooOld` (erro tipificado, nunca leitura errada) | Nunca erra para snapshots antigos (retenção preservada), mas sem borne superior — risco de disco |
| 7 | **Multi-node** (Montanha-Store): `Ok` ⇒ maioria aplicada (I-MAJ); Strong fail-closed sob dual-leader (I-RD); failover sem divergência (I-HA); TX cross-range 2PC atômica com restore de preimage (I-TX); DCS replicada (I-DCS) | Sim, com testes nomeados por invariante (`docs/montanha-invariants-and-tests.md`) | **Não existe** — RocksDB é single-process por design |
| 8 | **`#![forbid(unsafe_code)]`, clean-room Rust puro** | Sim | C++ (com unsafe/UB inerente à linguagem; não é claim de bug, é de classe de risco) |
| 9 | **Fault injection como produto** (seams `Env`/`Clock`/`Rng`/`Host`; FailingEnv, sync mentiroso, short-write, DST por seed) | Sim — as garantias são testáveis sob falha determinística | FaultInjectionTest existe em código de teste, não é seam de produto; sem cultura de simulação determinística equivalente |
| 10 | Artefatos de verificação formal (Verus) para componentes-chave | `crates/*/verus/` (parciais — **não** é prova do engine todo) | Nenhum |

### 2.3 Onde o RocksDB é mais forte (sem pinga de marketing)

| Área | Detalhe |
|---|---|
| Transações in-process | TransactionDB **pessimista** (2PL, detecção de deadlock), **2PC** Prepare/Commit, políticas WriteCommitted/WritePrepared/WriteUnprepared, OptimisticTransactionDB. PedraDB tem single-writer TX + OCC (`begin_occ`) — menor, sem lock manager |
| Concorrência de escrita | Multi-writer real (insert concorrente em memtable, two_write_queues, write groups paralelos). PedraDB serializa writers no fsync do WAL (linearizável, QPS single-writer class) |
| Maturidade de campo | Décadas de produção (Meta+comunidade); nosso próprio doctrine proíbe claim de paridade (`docs/robustness-vs-rocks-pebble-fdb.md`) |
| Recuperação fina de erros | Severidades kSoft/kHard/kFatal + listeners + auto-resume + quarentena de files; PedraDB tem binário cerca/reopen |
| Colunistas/feature surface | Column families atômicos cross-CF, merge operators, user-defined timestamps, wide columns, secondary/follower read-only, FIFO/universal compaction, subcompactions, SST ingestion, rate limiter, dois formatos de checksum (CRC32C/XXH3) |

---

## 3. Veredicto curto

- No **contrato de durabilidade local**, PedraDB = RocksDB em config default correta, e **mais forte que o default de fábrica do RocksDB** (sync ligado vs desligado; fence incondicional vs condicional; fail-closed em corrupção vs point-in-time salvage).
- Em **ricosura transacional local e throughput multi-writer**, RocksDB segue na frente — por design nosso (kernel pequeno).
- Em **tudo que é multi-node**, PedraDB oferece garantias que RocksDB não pretende ter (maioria, fencing de leitura, 2PC cross-range, HA, DCS).
- Em **testabilidade sob falha e determinismo**, o PedraDB é estruturalmente melhor posicionado (seams de produto + DST).
- **Não** reivindicar paridade de maturidade de campo — isso é doctrine do repo.

## 4. Footgun #1 de produção: blast radius do fail-stop (aberto)

**Status:** maior footgun operacional conhecido do PedraDB para produção testada
em batalha. Documentado 2026-08-17; decisão de design **aberta** —
`docs/open-items.md` §2.6.

A doutrina fail-stop (nunca silent-wrong) é o acerto e não se negocia. O custo
dela **hoje** é o raio de explosão: qualquer falha local — um fsync que falha
(ENOSPC transitório, hiccup de controladora), um registro corrompido no meio do
WAL — derruba **o banco inteiro** até close+reopen; com a escalada CORRUPTLOG
(RFC-0038), eventos repetidos recusam o open até intervenção. O menor defeito
tem o nó inteiro como vítima.

Quem decide depois do erro, por sistema (RocksDB = fontes §5; demais =
conhecimento estabelecido):

| Sistema | Quem decide depois do erro | Risco do default |
|---|---|---|
| LevelDB | Host (erro devolvido, handle segue) | Perigoso: continua sobre meio falhando sem contrato nenhum |
| SQLite | Host (TX aborta, handle segue) | Médio: a TX falha certo, mas o host pode insistir às cegas |
| RocksDB | Engine cerca **+ política exposta**: severidades soft/hard/fatal, listener pode rebaixar severidade, auto-resume com contagem, `DB::Resume()` manual, modos de recovery de WAL (point-in-time / salvage) | Baixo por default, mas `paranoid_checks=false` é a porta de fuga **sem tipagem** |
| **PedraDB hoje** | Engine cerca, sem escolha: reopen é o único caminho | **Mínimo de silent-wrong, máximo blast radius — zero política** |

**Por que a flexibilidade do RocksDB existe (é lição, não defeito):** ninguém
quer que um banco inteiro caia porque uma região (arquivo, CF, registro)
corrompeu. Disponibilidade sob falha **parcial** é requisito de produção
batalhada: o operador precisa poder dizer "quarentena esse arquivo e serve o
resto" ou "recupera até o ponto consistente e segue" — **sabendo o que foi
perdido**. O que o RocksDB faz mal é expor isso sem tipação
(`paranoid_checks=false` rebaixa silêncio de dúvidas); o que faz bem é
reconhecer que **fenced ≠ bricked**.

**Posição (veredito 2026-08-17 — não é defeito inerente):** PedraDB é uma
**primitiva**. Kernel não adivinha política de disponibilidade: cercar tudo e
devolver a decisão ao gerenciador é a postura correta para esse layer — o mesmo
motivo pelo qual LevelDB/SQLite também não decidem por conta própria. Hoje o
gerenciador é o Montanha (o cluster failover/redirect já existe). O item fica
registrado como **avaliação futura**, não como dívida imediata: reavaliar quando
existir consumidor single-process em produção que precise de blast radius menor
que o DB inteiro.

**Direção aceitável (sem relaxar garantia):** mecanismo fail-closed permanece no
kernel; política vira escolha explícita e tipada do gerenciador — severidades
tipadas (retryable/hard/fatal), hook no seam `Host` (testável em DST),
`recover_from_fence()` assistido (equivalente ao `Resume()`), e modos de
recovery declarados pelo operador (quarantine+serve-healthy, point-in-time com
relatório do descartado). Toda relaxação é opt-in e o piso fail-closed segue
sendo o default. Proibido por doctrine: flag que continua escrevendo sem o
gerenciador reconhecer a incerteza — isso recria o `paranoid_checks=false`
sem tipagem, e o silent-wrong volta pela porta dos fundos.

## 5. Fontes verificadas nesta sessão

- RocksDB `include/rocksdb/options.h` — `WriteOptions::sync` (default false), `WALRecoveryMode` (4 modos), `paranoid_checks` (default true)
- RocksDB `db/db_impl/db_impl_write.cc` — `WriteToWAL`/`AddRecord`, `WriteStatusCheck(WALOnly)`, `WALIOStatusCheck`, `PreprocessWrite` gate por `IsDBStopped`
- RocksDB `db/error_handler.h` / `db/error_handler.cc` — mapas de severidade (`kWriteCallback+kIOError+paranoid → kFatalError`; `kNoSpace → kHardError`), `is_db_stopped_`, quarentena, caso `manual_wal_flush+wal_related`
- RocksDB `include/rocksdb/db.h` — `Resume()` ("may or may not succeed"), `Close()` ("This will not fsync the WAL files"), `Open` LOCK
- RocksDB `include/rocksdb/listener.h` / `status.h` — `BackgroundErrorReason`, severidades, `kIOFenced`
- PedraDB — `crates/pedradb-core/src/db.rs` (default `sync: true`, fence em `4117-4124`/`4195-4203`, `ensure_not_fenced`), `src/error.rs` (`DurabilityFenced`, `CorruptionEscalated`, `Crc` cobre length+type+data), `src/corrupt.rs` (CORRUPTLOG, ⚠️ não commitado), `src/wal/reader.rs`, `docs/usage.md`, `docs/montanha-invariants-and-tests.md`

### Limitações deste relatório
- Wiki "Background Error Handling" do RocksDB não renderizou via fetch; a semântica de resume está citada do código + docstring do `DB::Resume()`.
- Comportamento de Postgres/SQLite/etcd/LevelDB é conhecimento estabelecido, não fonte primária desta sessão.
- PedraDB avaliado no **working tree atual** (inclui RFC-0038 CORRUPTLOG não commitado; se o commit mudar, revisar seção 2.2 linha 4).
