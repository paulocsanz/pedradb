# PedraDB robustness vs RocksDB, Pebble, FoundationDB

**Status:** honest comparison (lab + LEDGER, not marketing)  
**Updated:** 2026-08-12 (feature shape: leveled LSM, scan, ConcurrentDb, lz4, checkpoint; World Net+PeerMsg; competitor method + residual walls)  
  **Monorepo mirror:** `determinismo/pedradb-dst/ROBUSTNESS.md` (retired stale claims table §1.1)  
**Complements:** [`positioning.md`](positioning.md), [`switch-justification-bar.md`](switch-justification-bar.md), [`plan-limitations-and-failure-modes.md`](plan-limitations-and-failure-modes.md), [`rocksdb-critiques-and-improvements.md`](rocksdb-critiques-and-improvements.md), [`montanha-vs-foundationdb.md`](montanha-vs-foundationdb.md)  
**Hunt source of truth:** `determinismo/pedradb-dst/findings/LEDGER.md` (outside this repo)

---

## Resposta curta

**PedraDB ainda não é um competidor *field-mature* de RocksDB, Pebble ou Redwood/FDB.**

Avança em **método de competidor** (contratos de durabilidade, hunts LEDGER, multi-Raft Queued exchange + failover de lab, silent-wrong=0 sob schedules densos, TX all-or-nothing). **Não** tem — e este doc **não** claima — anos de abuso multi-tenant em cloud, write-amp Rocks-class, PITR industrial, nem Simulation FDB.

**Síntese:** lab + reliability culture no caminho certo; **menos** robusto que Rocks/Pebble/FDB como primitiva de infraestrutura de produção.

O gap dominante continua **field time + escala + ops de guerra + simulação de sistema**, não “faltou um CRC”.

---

## O que “robusto” significa aqui

| Eixo | O que importa |
|------|----------------|
| **Durabilidade** | Put Ok + crash/power → dado ainda lá |
| **Integridade** | Bitrot / torn write → fail-stop, não silent wrong |
| **Disponibilidade** | Compact/flush/recovery sob ENOSPC, EIO |
| **Consistência multi-key** | TX all-or-nothing |
| **Escala / performance** | Níveis, cache, compaction I/O, multi-writer |
| **Ops** | Backup, versioning, migration, tooling |
| **Maturidade** | Anos de produção + fuzz + correções |

PedraDB ganha pontos nos **três primeiros** (e no desenho). Perde nos **quatro últimos**.

---

## PedraDB hoje (com o que o hunt fechou)

### Pontos fortes (reais)

1. **Contrato de durabilidade explícito**  
   `sync=true` → WAL fsync antes de Ok; TX multi-key em um record WAL; torn tail descartado, não half-TX.

2. **Integridade on-disk com falhas já fechadas (LEDGER)**  
   - F1–F6: SST tmp+rename; bounds anti-OOM; CRC SST/MANIFEST/WAL; length no checksum; torn empty WAL  
   - F13–F14: WriteRecord op-count bound; orphan Middle/Last fail-stop  
   - F18–F21: auto-flush não invalida ack; wide SST nums; auto-compact + snapshots; flush MANIFEST rollback  
   - Camadas: F7–F12, F15–F17 (DCS/HTTP/Raft); F22–F28 (Montanha-Store)

3. **Seam de falha de primeira classe**  
   `Env` + `FailingEnv` / `RecordingEnv` (espírito RBS/`FailingMedia`) — história limpa para DST determinístico. RocksDB tem fault injection; o story do Pedra é mais legível no monorepo.

4. **`#![forbid(unsafe_code)]` no core**  
   Menos superfície de UB; trade-off de performance e de mmap/io_uring.

5. **Montanha-Store em cima**  
   Majority commit, strong vs local read, meta raft durável, compact + membership remove; **Queued RPC** elect+put+**failover** + InstallSnapshot via `PeerMsg`. Lab Net path: `determinismo/pedradb-dst/world` (InProcessNet). Ainda **MVP** (sem TCP multi-Raft de produção).

6. **Reliability culture (competitor method)**  
   Denser put/delete/TX/flush/compact crash schedules with **silent_wrong=0** vs model; multi-key TX Ok→reopen full; uncommitted TX → no half-visible; SST bitflip fail-closed.

### Feature shape (shipped 2026-08 — lab, not field)

| Feature | PedraDB hoje | vs Rocks/Pebble |
|---------|--------------|----------------|
| **Leveled LSM** | ✅ L0 flush + compact N→N+1 (subset); MANIFEST v2 levels | ✅ maduro / multi-policy |
| **Streaming range** | ✅ `Db::scan` / `scan_at` heap-merge (public path) | ✅ iterators de guerra |
| **Concurrent access** | ✅ `ConcurrentDb` (RwLock; puts serialised) | ✅ multi-thread write path |
| **SST compression** | ✅ v4 lz4 blocks; v1–v3 still open | ✅ vários codecs + dict |
| **Table/block cache** | ✅ `TableCache` + `BlockCache` + stats | ✅ block cache maduro |
| **Range delete** | ✅ `delete_range` tombstones + compact GC | ✅ DeleteRange maduro |
| **Checkpoint / verify** | ✅ open-as-DB checkpoint; verify fail-closed CRC | ✅ + tooling de field |
| **Queued multi-Raft + failover** | ✅ lab Queued elect/put/leader-loss majority | ✅ production cluster |

### Residual walls (honest — still not field-mature competitor)

| # | Gap | Status | Notes |
|---|-----|--------|-------|
| 1 | **Field maturity** | ❌ unaddressable by code | Months + LEDGER F1–F28 ≠ decade of cloud abuse |
| 2 | **Concurrency amp** | 🟡 lab done | Group commit + dual-mem pipeline on `ConcurrentDb`; still not Rocks multi-mem writer class |
| 2b | **Value-log GC** | ✅ P0 done | `Db::compact_vlog` crash-safe rewrite (RFC-0016); threshold still opt-in |
| 3 | **Compaction & multi-TB fleets** | 🟡 shape done | Leveled N→N+1 + streaming `scan`; policy still basic; no multi-TB field proof |
| 4 | **Ops PITR/migration** | 🟡 local suite | `pedradb-ops`: base backup, WAL ship, PITR restore, format migrate + CLI; **not** multi-region cluster backup |
| 5 | **Distributed substrate** | 🟡 MVP | World seed→Net∧disk∧clock + Queued PeerMsg; **not** production TCP multi-Raft / FDB Simulation |
| 6 | **io_uring / advanced I/O** | 🟡 shipped opt-in | `pedradb-io-uring::IoUringEnv` (Linux write+fsync); macOS/dev → POSIX fallback |

**Competitor posture:** advance reliability *method* (hunts, silent-wrong=0, multi-Raft exchange). **Do not** claim field-mature Rocks/Pebble/Redwood production parity.

---

## Comparação direta

### vs RocksDB

| | PedraDB | RocksDB |
|--|---------|---------|
| **Maturidade** | Meses de desenho + hunts | ~10+ anos, Meta/produção massiva |
| **Modelo** | Single-writer LSM clean-room | Multi-threaded, columns, TX opcional |
| **Crash/recovery** | Contrato claro + fixes F1–F6, F13–F21 | Maduro, com histórico real de bugs e fixes |
| **Fault injection / DST** | Seam `Env` nativo no desenho | Existe, mais “ao lado” do monólito C++ |
| **Performance/escala** | Não compete ainda | Default industrial |
| **Unsafe / complexidade** | Rust, forbid unsafe | C++, enorme superfície |

**Veredito:** RocksDB é mais robusto como produto e como bet de produção. PedraDB é mais simples de raciocinar e de caçar falhas de durabilidade em laboratório. **Não é substituto drop-in.**

### vs Pebble (Cockroach)

Pebble = RocksDB-class em Go, com lições de Cockroach (separação de concerns, menos footguns C++, integração com raft do CRDB).

- Mais robusto que PedraDB em produção e features.
- PedraDB tem range delete + leveled *shape* de lab; ainda não tem o peso de Pebble em multi-TB, policy e SSTable battle-tested.
- Em clareza de “o que fsync garante”, os dois podem ser honestos; Pebble tem mais edge cases já vistos em cluster.

### vs storage do FoundationDB

FDB não é “um LSM embutível genérico” no mesmo sentido:

- Storage node + transaction system + determinism / simulation (buggify) é o diferencial.
- A robustez do FDB vem mais de **simulation testing do sistema** do que só do format on-disk.
- PedraDB tem o início de uma cultura DST (`FailingEnv`, sweeps, LEDGER); FDB tem anos de “simule o universo e quebre tudo”.

**Veredito:** primitiva + controle de FDB é mais robusta como base de sistema distribuído. PedraDB+Montanha-Store **aspira** a essa camada (store multi-Raft + DCS on store), mas o MVP ainda é in-process e jovem.

### vs Badger / sled / outros embed

Aí PedraDB compete na mesma liga conceitual (embed KV Rust/Go, single process). Em durabilidade consciente e hunts documentados, pode estar à frente de vários embeds “bons o bastante”. Ainda atrás de qualquer um com deploy massivo.

---

## Escala de honestidade (hoje)

```text
Prova de produção / guerra
  FDB cluster ████████████████████
  RocksDB      ███████████████████
  Pebble       █████████████████
  PedraDB      ███░░░░░░░░░░░░░░   ← bom lab + contratos; pouco field time

Clareza de contrato fsync + seams de teste
  PedraDB      ████████████████░░  ← forte no desenho
  FDB (sim)    ████████████████████
  Rocks/Pebble █████████████░░░░

Adequação “substituir Rocks no hot path de DB cloud”
  PedraDB      ░░░░░░░░░░░░░░░░░░  ← não
```

---

## Matriz residual (eixo justo: crash / bitrot / ENOSPC)

Comparativo mais justo que benchmark de marketing — alinhado ao LEDGER §A/D.

| Classe | PedraDB hoje | Rocks / Pebble | Notas |
|--------|--------------|----------------|-------|
| **Put Ok + crash → dado lá** (`sync=true`) | ✅ contrato + F18 | ✅ maduro | WAL fsync antes de Ok; auto-flush não invalida ack |
| **Torn tail / half-TX** | ✅ descartado / fail-stop (F4, F14) | ✅ (muitos edge cases já vistos) | Pedra TX = 1 record WAL |
| **Bitrot silent wrong** | ✅ fail-stop F2–F5, F9, F13 | ✅ (+ histórico de bugs reais) | CRC SST/MANIFEST/WAL/WriteRecord |
| **Partial SST bloqueia open** | ✅ F1 tmp+rename | ✅ | |
| **ENOSPC / EIO no path de write** | ✅ lab (`FailingEnv`, fail_after CLEAN pós-fix) | ✅ + field lore | Pedra: cobertura de lab, zero field time |
| **Lying fsync (kernel mente)** | ⚠️ in-process `RecordingEnv`; det_io **BLOCKED** no macOS | ⚠️ também depende do OS | Ninguém “prova” power-loss sem hardware/TCG |
| **Multi-writer concurrent** | 🟡 `ConcurrentDb` serialised puts | ✅ | Feature exists; not Rocks throughput |
| **Disponibilidade sob full disk em compact longo** | ⚠️ LSM leveled básico; sweeps parciais | ✅ pacing/ops maduros | |
| **Range / multi-TB shape** | 🟡 streaming `scan` (lab-tested 3k+ keys) | ✅ multi-TB field | form shipped; not field-scale |
| **Backup / restore / migration** | 🟡 checkpoint open-as-DB | ✅ ecossistema | |
| **Simulação de sistema (cluster)** | 🟡 World P1/P2 (Net+Env+clock) + DST kernel | FDB: anos de buggify | Paridade de *método* parcial; não FDB Simulation |
| **Histórico de produção** | ❌ meses + hunts | █ 10+ anos | o gap dominante |

### LEDGER residual (não é “perdemos pro Rocks no CRC”)

**FIXED (caça real, amostra):** F1–F6, F13–F14, F18–F21 (core); F7–F12, F15–F17 (deps/raft); F22–F28 (store).

**BLOCKED / tool gaps (LEDGER §D):**

| Item | Blocker |
|------|---------|
| det_io lying-fsync na macOS host | precisa Linux/TCG ou dylib attach |
| live-rocksdb oracle | C++ toolchain / feature não default |
| DCS multi-node durable leases | lease SM ainda não no log Raft multi-node |
| Store network multi-Raft | World InProcessNet + PeerMsg; TCP prod adapter open |
| WAL ship `pull` full-delta alloc | pode multi-GB se WAL enorme |
| Raft network auth | sem mTLS/shared secret em bind público |

**LIMITATION de produto (não bug de CRC):**

- concurrent puts via coarse lock (not Rocks multi-writer amp)  
- compaction policy basic (not universal / dynamic leveling)  
- no multi-TB field proof despite streaming shape  
- checkpoint only — not full PITR/ops suite  
- Montanha-Store não é cluster de produção  
- zero production field history (dominant gap) 

Revalidar esta tabela sempre que o LEDGER mover status de F* ou §D.

---

## Quando PedraDB é a escolha certa

- Embed **single-writer**, controle total do código, Rust, Apache-2.  
- Queres oracle + `FailingEnv` + campanhas (determinismo/DST) como no resto do monorepo.  
- Camada tipo Montanha-Store / DCS on store onde o **produto é o sistema em cima**, e o kernel é seu.  
- Aceitas que robustez = **processo de hunt contínuo**, não “já ganhou da indústria”.

## Quando não é

- Precisas de multi-TB, multi-writer, compression, secondary index de guerra.  
- Precisas de “ninguém foi demitido por escolher X” (Rocks/Pebble/FDB).  
- Precisas de cluster storage já battle-tested **hoje**.

---

## Uma linha para colar

**PedraDB compete em honestidade de contrato e caça a falhas em laboratório; Rocks/Pebble/FDB competem (e ganham) em robustez de infraestrutura.**
