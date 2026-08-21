# Certeza × disponibilidade — a escolha se sustenta? (com o mercado na mesa)

**Status:** análise de decisão, 2026-08-20
**Pergunta que originou:** priorizar corretude (fail-closed) sobre
disponibilidade está certo, se isso exige S3+PITR conectado ou um disco
enorme? Como os concorrentes fazem? "DB feito sobre RocksDB já aceita perde
dados por falta de fsync e é construída para se recuperar" — isso não
invalida nossa posição?
**Companheiros:** [rocksdb-vs-pedradb-guarantees.md](rocksdb-vs-pedradb-guarantees.md) §4,
[RFC-0046](rfc/0046-mvcc-history-tiering-s3.md) (storage),
[RFC-0045](rfc/0045-multi-writer-async-5x.md)
**Marcadores:** claims RocksDB = verificados por arquivo do upstream
(guarantees doc, 2026-08-17). Demais sistemas = **conhecimento estabelecido**
(não reverificados nesta sessão — não promover a "verificado" sem fonte).

---

## 1. São dois eixos, não um

Chamar de "certeza vs disponibilidade" mistura duas decisões independentes:

- **Eixo A — durabilidade no Ok** (power loss): o que o DB perde de writes
  *já acked* quando a máquina cai.
- **Eixo B — disponibilidade na falha** (fsync falho, corrupção no meio do
  WAL): quando o meio falha *de formas parciais*, o DB cerca ou segue?

A pergunta difícil é a B. A A já está decidida e é o produto.

## 2. O mercado, eixo a eixo

| Sistema | A: perde write acked? (default) | B: o que faz na falha parcial | Campo |
|---|---|---|---|
| **Pedra** | **Não** (G1: fdatasync antes do Ok) | Cerca sempre; corrupção → recusa o open | nós |
| RocksDB (default `sync=false`) | **Sim** (WAL no page cache) | Point-in-time recovery e segue; salvage; auto-resume; fence só com `paranoid_checks` | verificado |
| → TiKV / kvrocks / MyRocks / SurrealDB | Herdam do Rocks salvo config explícita | Idem + a camada de cima re-replica (Raft) | estabelecido |
| Postgres | Não por default (`synchronous_commit=on` faz WAL durável no commit) | **PANIC** no flush fail — fail-stop como nós; PITR via basebackup + WAL archive (S3) | estabelecido |
| etcd (boltdb) | Não (fsync por txn) | **Panic** no fsync fail — fail-stop; a saúde vem do cluster (quórum Raft) | estabelecido |
| FoundationDB | Por nó: sim; a durabilidade é o **quórum de tLogs** (replicação), não o disco local | Processo crasha; o cluster re-replica e segue | estabelecido |
| CockroachDB | Por nó: pode; durabilidade = **quórum Raft** | Nó cai → re-replicação; o engine (Pebble) é só o local | estabelecido |
| MongoDB (WiredTiger) | **Sim, até ~100 ms** de janela de journal por default (`j:true` aperta) | Modo repair; disponibilidade antes de certeza | estabelecido |
| Cassandra/Scylla | **Sim** — commitlog `periodic` default (janela de ~10 s; `batch` aperta) | Scrub/skip de segmentos corrompidos; re-replicação | estabelecido |
| Redis | **Sim, até 1 s** (AOF everysec default) | Falha de persistência não derruba o processo | estabelecido |
| SQLite | Não (journal/WAL com fsync default) | TX aborta; handle segue | estabelecido |

**O padrão que emerge:** existem dois acampamentos.

1. **Replicação-primeiro** (FDB, Cockroach, Cassandra, o ecossistema
   RocksDB-em-cluster): a durabilidade mora **acima** do nó; o nó pode ser
   lossy e leniente porque o cluster re-replica. *O argumento do usuário é
   exatamente este acampamento — e está certo para ele.*
2. **Nó-primeiro** (Postgres, etcd, SQLite): a durabilidade mora **no nó**;
   o nó é estrito (fsync por commit, fail-stop em falha), e a alta
   disponibilidade é montada por fora (streaming replication, cluster Raft
   de etcd, backup+WAL archive).

O RocksDB é um caso especial: **não escolhe** — é biblioteca, default
leniente, e deixa o DB de cima decidir. O TiKV escolheu replicação;
o Postgres, se embutisse um LSM, escolheria nó-primeiro.

## 3. Onde nós estamos — e o que o argumento do usuário acerta e erra

**Acerta:** DBs sobre RocksDB aceitam perda e são construídas para
recuperar. Se o Pedra for consumido **sempre sob um gerenciador
replicado** (Montanha), o acampamento replicação-primeiro bastaria: um nó
cercado vira failover, e certeza-vs-disponibilidade é política do cluster.

**Não invalida, por três motivos:**

1. **G1 (eixo A) não é troca por disponibilidade — é o produto.** A tese
   inteira é "mais durabilidade **e** mais rápido que o Rocks que corre em
   produção" (apply_mc4 2.8× pagando fdatasync; RFC-0041). No eixo A, o
   default do mercado (perder write acked) é o **bug** que vendemos contra;
   Postgres/etcd/SQLite provam que estrito-local é postura de produto
   viável há décadas. O argumento "todo mundo aceita perda" descreve o
   acampamento 1, não um consenso.
2. **O papel nosso é primitiva local.** Pedra compete pelo lugar do
   RocksDB/Redwood/Pebble — o substrato. Um substrato que adivinha política
   de disponibilidade força TODOS os gerenciadores a herdarem a escolha
   dele. Cercar (fail-closed) e devolver a decisão tipificada ao host é o
   design correto nesse layer; o mesmo motivo pelo qual não embutimos Raft.
3. **A diferença real está no eixo B, e lá a crítica procede em parte:**
   nosso fail-closed **exige pareamento operacional** para ser viável —
   failover (Montanha), ou backup/PITR (RFC-0046), ou aceitar downtime
   manual. O Rocks `kPointInTimeRecovery` "segue servindo" **perdendo
   dados que o usuário não sabe que perdeu**; o Postgres PANIC **igual a
   nós** e ninguém chama o Postgres de fraco — a diferença é que o
   ecossistema Postgres tem WAL archive e réplica prontos na primeira
   página. Nossa falha hoje não é a escolha; é **faltar o pareamento
   óbvio** (o RFC-0046 é exatamente isso).

## 4. Requisitos operacionais por garantia (o que precisa existir para cada promessa valer)

| Promessa | Vale sozinha? | Precisa de |
|---|---|---|
| Write acked sobrevive a power loss (G1) | **Sim** | nada |
| Nunca ler errado (fail-closed em corrupção) | Sim, mas **cara**: nó parado | failover (Montanha) **ou** PITR/backup (RFC-0046) **ou** tolerância a downtime manual |
| História/MVCC/PITR | **Sustentável desde 2026-08-21** (RFC-0046 P0): retention default bounded (`Window(24 h)`) + archive local com cap e watermark tipificado — disco ≈ live set + janela | horizon/archive já no kernel; S3 (P1) deixa a história barata e sobrevive a perder o disco local |
| Fence após fsync falho | Sim (cerca) | reopen; política de severidade tipada é a evolução (abaixo) |

## 5. Veredito

1. **Eixo A (durabilidade no Ok): estamos certos, e não é gosto — é o
   produto.** O custo está medido e pagável; o mercado leniente é o alvo,
   não o padrão a copiar. O precedente é Postgres/etcd/SQLite.
2. **Eixo B (fail-closed na falha): a escolha se sustenta como kernel,
   com duas condições explícitas** — (a) o pareamento operacional tem de
   existir e ser óbvio (RFC-0046 + Montanha; sem isso, single-process
   embutido vira "bricked até intervenção" e a crítica procede);
   (b) a política de disponibilidade deve migrar para o host **tipada**
   (severidades retryable/hard/fatal + hook no seam `Host` + modos de
   recovery declarados — a direção já desenhada no guarantees doc §4),
   nunca uma flag que continua escrevendo sem reconhecer incerteza (isso
   recriaria o `paranoid_checks=false` sem tipagem). **Concretizado em
   2026-08-21: o primeiro host é o próprio `rocksdb-compat` —
   [RFC-0047](rfc/0047-compat-dropin-failure-profile.md) dá à face
   drop-in o perfil de falha do RocksDB (PointInTime + `resume()`,
   tipados e reportados, escalada CORRUPTLOG preservada) sem tocar o
   piso fail-closed do kernel.**
3. **Se um dia o consumidor dominante for embedded single-process sem
   backup nem réplica**, a posição 2(b) deixa de ser suficiente e um modo
   leniente *declarado* (estilo point-in-time com relatório do descartado)
   passa a ser requisito de produto — decisão de roadmap, não de kernel.
4. O que **não** se sustentava até 2026-08-21: retenção default
   ilimitada no SSD. Era bug de sustentabilidade, não posição
   filosófica — **fechado pelo RFC-0046 P0** (horizon bounded
   default `Window(24 h)` + archive local com cap; estouro do cap
   avança o watermark com `SnapshotTooOld` tipificado, nunca
   destrói silenciosamente; F20 vira opt-in `HistoryHorizon::All`).

## 6. Fontes

- Verificado (arquivo do upstream, 2026-08-17): `options.h` (`sync`
  default false), `db_impl_write.cc`/`error_handler.cc` (severidades,
  fence condicional, point-in-time) — ver
  `rocksdb-vs-pedradb-guarantees.md` §5.
- Conhecimento estabelecido (marcado na tabela; não reverificar = não
  citar como verificado): Postgres PANIC/`synchronous_commit`; etcd
  boltdb fsync+panic; FDB quórum de tLogs; CRDB quórum Raft;
  MongoDB 100 ms; Cassandra periodic/gc_grace; Redis everysec; SQLite.
- In-repo: `robustness-vs-rocks-pebble-fdb.md`,
  `references/cockroachdb-why-rocksdb.md` (papel do storage engine:
  Atomicidade + Durabilidade no nó, distribuição acima),
  `architecture-refined.md` (Pedra = primitiva local),
  `open-items.md` §2.6 (blast radius).
