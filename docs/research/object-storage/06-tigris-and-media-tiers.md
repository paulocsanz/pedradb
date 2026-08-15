# Tigris, six jobs of object storage, and media tiers

**Status:** research note, 2026-08-15. Not a product RFC.  
**Primaries:** [`../../references/tigris-architecture-primaries.md`](../../references/tigris-architecture-primaries.md)  
**Montanha mapping:** [`07-montanha-as-tigris-control-plane.md`](07-montanha-as-tigris-control-plane.md)  
**Earlier (partially wrong on Tigris data plane):** [`../../object-storage-as-substrate-possibility.md`](../../object-storage-as-substrate-possibility.md) — superseded for Tigris; this file is the correction.

---

## How Tigris works

Tigris is **not** an erasure-coding cluster. FoundationDB is the brain (name, version, where the bytes are, small objects). Large bytes go to **block storage** (NVMe on Fly + another object store off-network) in **n copies**. SSD is cache. Metadata is **pushed** to the world; bytes are **pulled** when someone reads. EC, when it exists, is **under** the backing store — not in the product they document.

```
cliente S3
    │
    ▼
gateway stateless (qualquer região, anycast)
    ├─ SSD cache          → hit rápido (dado e/ou metadado)
    ├─ FoundationDB       → metadado sempre;
    │                       objeto pequeno INLINE no registro;
    │                       fila de replicação (o DB é a fila)
    └─ block storage      → n cópias do payload
         · local (NVMe Fly)
         · remoto (outra região / outro provider)
```

### PUT em São José

1. Bytes → block store (e talvez SSD).
2. Metadado (nome, tamanho, Regions: `[SJC]`) entra no FDB **na mesma transação** que o enqueue da fila.
3. Um worker empurra **só o metadado** para os outros clusters FDB (segundos).
4. Chicago já sabe que o objeto existe e onde está. Ainda **não** tem os bytes.

### GET em Chicago

1. Lê o metadado local (`Regions: SJC`).
2. Puxa os bytes do block store de SJC, grava no SSD e no block local.
3. Actualiza `Regions: SJC, ORD` e empurra de novo.

Por isso um DC “levando um meteoro” ainda serve a foto: o metadado já saiu, e o block store **não** está no mesmo provider que o frontend (Fly diz isto com todas as letras: NVMe + object store *off-network*). Anycast esconde a região morta.

**Consistency:** o default é esse push **assíncrono** de metadado — janela de segundos em que outra região pode 404. Quase ninguém liga. Quem liga liga “global strong consistency”: uma região líder, o resto faz proxy (mais lento).

**Residency / GDPR:** header `X-Tigris-Regions` prende os bytes. GET de fora é reverse proxy, não cópia. (Product claim; not re-verified against the header docs this session.)

**Accelerate:** PUT também empurra bytes (CDN eager). O default é pull. Matches official **Cache on Write** vs **Cache on Read**.

Sources: [architecture](https://www.tigrisdata.com/docs/concepts/architecture/), Fly [public beta](https://fly.io/blog/tigris-public-beta/) + [customer story](https://fly.io/customers/tigris/), [QuiCK](https://www.foundationdb.org/files/QuiCK.pdf), [forking deep dive](https://www.tigrisdata.com/blog/bucket-forking-deep-dive/).

---

## What FoundationDB actually does

It is **not** the disk of the 5 GB video.

| No FDB | Fora do FDB |
|--------|-------------|
| tenant / bucket / key / versão | payload grande |
| etag, size, user metadata | n cópias em block |
| `Regions: [...]` (ponteiro) | SSD cache |
| objetos pequenos **inline** (Fly compara com Redis) | |
| índices secundários, CAS, transação | |
| a fila de replicação (padrão QuiCK / CloudKit) | |

Why FDB and not Postgres+Kafka:

- serialização estrita no keyspace inteiro (classe Spanner);
- índice secundário = outro KV **na mesma transação**;
- simulation testing da Apple;
- versionstamp → ordem total → a fila é um range scan por tempo;
- PUT + “avisa o mundo” **não** é two-phase entre dois sistemas.

Layout (forking blog): nomes viram caminho ordenado; cada objecto é um WAL. Snapshot = um `u64` (`MaxUint64 - unix_nanos`). Fork = bucket filho que, no miss, recursa no snapshot do pai. **Não copia terabytes.** Isto só existe porque o KV é ordenado — é feature de **metadado**, não de EC.

**Inline cutoff (measured claim, not our policy):** Fly public beta says objects **≲ 128 KiB** become instantly global. Official architecture does not publish a byte cutoff. FDB’s own value limit is **100 KiB**. Do **not** write “Tigris inlines 1 MiB” — that 1 MiB figure is a *candidate Armazém policy*, not a Tigris primary.

**What I did not find in an official source:** Reed-Solomon profile, erasure set, CRUSH, “we rebuild a 16 TB HDD.” Architecture + Fly talk in **n copies** / NVMe + off-network object store. The second store, underneath, almost certainly does EC on HDD — Tigris does not publish that *they* do.

---

## Object storage is six jobs. EC is #5.

| # | Job | Tigris | MinIO / Ceph / B2 / Armazém-A |
|---|-----|--------|-------------------------------|
| 1 | API + auth | gateway global | igual |
| 2 | namespace | FoundationDB | xl.meta / omap / Postgres |
| 3 | objeto pequeno | inline no FDB | replica ou pack |
| 4 | quente | SSD cache | page cache / NVMe / CDN |
| 5 | bytes duráveis | n cópias + backing store | Reed-Solomon nos teus discos |
| 6 | colocar e mover | push metadado, pull byte | CRUSH / vault / heal |

Two families:

- **A — dono do disco:** MinIO, Ceph, Backblaze, Seaweed. Host some → reconstrói de *k* shards. Conta = teu HDD + tua NIC.
- **B — dono do control plane:** Tigris, R2 (Workers + Durable Objects + storage regional). Região some → metadado já está noutro sítio, puxa bytes. Conta = FDB/DO + NVMe + o object store de alguém.

S3 **para ti** é B. Por dentro a Amazon é A: milhões de HDDs, nós KV burros, Reed-Solomon, e o problema de verdade é calor (IOPS por disco; disco de 26 TB hoje). Warfield/FAST 2023: EC existe para poder usar esses HDDs sem o seek de 8 ms virar latência de cauda. (Cite as the usual FAST’23 talk; PDF not re-fetched this session.)

---

## SSD, HDD, NVMe — what the market actually uses

Not “object store = SSD”. IOPS vs $/GB.

| Camada | Mídia | Uso real |
|--------|-------|----------|
| RAM | DRAM | page cache, working set do FDB |
| NVMe | TLC/QLC | S3 Express; cache do Tigris; MinIO “all flash”; WAL/DB do Ceph; storage server do FDB |
| SATA SSD | TLC | índice, objeto pequeno replicado |
| HDD | 16–26 TB CMR | S3 Standard/IA (capacidade); vaults Backblaze 17+3; data pool EC do Ceph |
| HDD denso / SMR | 20 TB+ | backup, IA |
| fita / Glacier Deep | LTO | arquivo com restore de horas |

Product names are not media:

- S3 Standard *looks* like SSD; it is HDD + EC + a huge cache. SSD is metadata and Express.
- S3 IA is the same disk, different bill (min 128 KiB, retrieve fee).
- Glacier Instant is still disk, packed colder.
- Glacier / Deep is a restore API; tape or offline HDD.
- Tigris Standard = NVMe + backing store + FDB. Their Glacier: restore ~1 h; media not published (not re-fetched).
- B2 = consumer HDD, 17+3.
- Serious Ceph = replicated SSD pool for RGW index + EC HDD pool for data. Their docs ask for that.

Rule that does not break: **metadata and the first hot/small byte go to flash. Hot/cold capacity goes to HDD+EC — or you pay 3× NVMe and lose to B2.**

Tigris can “not have HDD” because it does **not** compete on $/GB. It competes on fast small objects + a global bucket. HDD+EC would make them MinIO with a passport.

---

## What this changes for Armazém (caixote RFC 0181, not in tree as of 2026-08-15)

Do **not** become Tigris. Different job.

Steal only this:

1. Metadata **off** the EC disk (already the intended RFC shape).
2. Inline / 3-replica below the **cluster value limit** (~100–128 KiB is what Tigris/FDB actually do; 1 MiB is a Pedra-embed option, not their published cutoff).
3. NVMe cache in front of HDD+EC, when the cell is cheap.
4. Class FAST = n copies on NVMe; STANDARD = EC on HDD. Same ladder as Azure WAS and caixote RFC 0091.

**Global metadata push is not P0** — we do not have 13 regions.

Ready-to-paste addendum: see § “Texto para RFC 0181” in [`07-montanha-as-tigris-control-plane.md`](07-montanha-as-tigris-control-plane.md).
