# RFC-0040: `fdatasync` always-on mais rápido que Rocks **async**

**Status:** in-progress  
**Updated:** 2026-08-17  
**Parents:** [0039](0039-apply-raftlog-scan-5x-rocks-sync.md) (5× vs sync), [0037](0037-apply-off-put-and-2x-pedra.md) (group commit + host worker), [0036](0036-tikv-rocks-2x-fdatasync.md) (G1 = `fdatasync` antes do Ok)

## Background

- Pedra no Ok **sempre** `fdatasync` (G1). Rocks default (`sync=false`) não. Isso **não** é um teto: o piso é por **ack/grupo**, não por chave.
- Arquitetura que paga um fd e mesmo assim passa o async: group commit (N ops / 1 fd), pipeline (encodar o próximo grupo enquanto o líder sinca), **uma** cópia do payload até o WAL, compact/flush fora do Ok, scan sem setup de 20 µs.
- **P0 shipped (`92eef56`):** `encode_into` + scratch no `WalWriter`; `WriteOp` move para a mem; count SST sem `Box<dyn>` e sem `user_key.clone()` por passo. Ainda há um memcpy lógico → frame (CRC precisa dos bytes). Bounds do cursor SST ainda viram `Bytes`.
- Coluna **async é obrigatória** em toda remesura (pedido permanente do dono). 5× vs sync continua no RFC-0039; este RFC é o ganho contra o Rocks que as pessoas correm.

## Problems This Solves

- **Problem:** “fd always-on ⇒ não dá para ganhar do async” trata o syscall como custo por chave. Não é.
- **Problem:** CPU à volta do fd (2 memcpy + alloc por write) é maior que o fd em batches de 32 KB. Cortar isso é vitória **mesmo** no cliente único.
- **Problem:** scan vs async = scan vs sync (leitura não sinca). Perder no p95 é o cursor SST, não o WAL.

## Proposed Solution

1. **P0 — uma cópia até o WAL + mem por movimento.** `encode_into` num scratch reutilizado; o writer reutiliza o buffer de fragmentação; `WriteOp` move para o memtable depois do append. Semântica e bytes no disco idênticos. Feed em memória fica ( `Bytes::clone` é refcount ); `changelog_interval=0` já não persiste no commit.
2. **P1 — amortizar o fd.** Group commit já existe no `ConcurrentDb`. Remesura **multi-cliente** apply/raftlog vs Rocks async: `qps_pedra / qps_async` com o mesmo N. Sem skip de sync; o Ok de cada membro ainda espera o fd do grupo.
3. **P2 — scan e pipeline.** Cursor de count sem `Box`; L0 drenado; pipeline de encode∥fsync no host (não no core) se o P1 ainda perder no MC.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | `fdatasync` antes de cada Ok (ou do Ok do grupo) | intocada |
| G2 | visibilidade = lookup / range_at | encode/mem iguais; count mesmo conjunto |
| G4 | adversarial | re-verde sem editar asserção |
| G5 | fence se sync falha | intocada |
| G6 | sem thread no core | P2 pipeline só no host |
| G8 | async **e** sync na tabela; sem misturar classes num “ganhámos do Rocks” |

## Delivery slices (mandatory)

### P0 — must ship first (CPU do write + scan setup; útil no cliente único)

- [x] **P0.1** RFC + Status vivo (este doc) — status: `done`
- [x] **P0.2** `WriteRecord::encode_into` + scratch no `WalWriter`; `commit_ops_with` / group commit sem clonar o `WriteRecord`; `WriteOp` move para a mem depois do WAL. Bytes no WAL = encode antigo. — status: `done`
- [x] **P0.3** Count SST sem `Box<dyn>` e sem `user_key.clone()` por passo (mesmo conjunto visível) — status: `done`

### P1 — amortizar o fd contra async

- [x] **P1.1** Harness MC (N=4) apply + raftlog vs Rocks **async** e vs sync; publicar as duas razões — status: `done` (`run_deps_clients`; finding [rfc0040-p11](../findings/rfc0040-p11/README.md): apply MC mediana **0.93×** async, raftlog MC 0.61× / run1 1.57×)
- [x] **P1.2** Sticky group (250 µs) para o 2º `write()` do apply não ir no fast path; `group_commit` move ops. Catch-up 200 µs medido e **rejeitado** (qps cai). Apply MC não fecha ≥1.0 de forma estável (0.54 / 0.97). — status: `done`

### P2 — scan vs async + pipeline

- [ ] **P2.1** `deps_scan` ≥ Rocks async da mesma run (async=sync nas leituras) — status: `todo`
- [ ] **P2.2** Pipeline encode∥fsync no **host** se P1.1 ainda perder no MC — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este doc | 2026-08-17 |
| P0.2 | p0 | encode_into + scratch + move-to-mem | done | este commit | 2026-08-17 |
| P0.3 | p0 | count cursor sem Box / sem clone | done | este commit | 2026-08-17 |
| P1.1 | p1 | MC vs async+sync | done | `run_deps_clients` + findings/rfc0040-p11 | 2026-08-17 |
| P1.2 | p1 | sticky group + catch-up sweep | done | findings/rfc0040-p12 | 2026-08-17 |
| P2.1 | p2 | scan ≥ async | todo | — | 2026-08-17 |
| P2.2 | p2 | pipeline host | todo | — | 2026-08-17 |

## Acceptance Criteria

- **Tests:** `encode_into` ≡ `encode`; recover depois de `apply_batch` grande; `rfc19_change_feed_*` verde; `count_borrowed_matches_streaming_all_windows`; `cargo test -p rocksdb-compat` adversarial sem editar asserção.
- **Telemetry:** remesura ycsb+deps com coluna sync **e** async (P1.1). P0 não exige bater o async sozinho (cliente único + 2 fd no apply).
- **Documentation:** este RFC + linha em `docs/open-items.md`. backend-only.
- **Screenshots:** none — backend-only.

## Out of scope

- Skip de `fdatasync` no Ok.
- Fundir os dois `write()` do apply num só fd (muda o schedule).
- 5× vs sync (RFC-0039).
- Thread no `pedradb-core`.

## Next run (handoff)

Abrir isto primeiro. Não reabrir a discussão “fd always-on impede ganhar do async” — o piso é por ack/grupo. Coluna **async obrigatória** em toda remesura. G1/G4/G6/G8.

### Ordem (não pular P1.1)

1. **RFC-0040 P1.1 — medir MC vs async + sync** (a fatia que falta de verdade).  
   `YcsbRunner::run_clients` só faz `ycsb_a` / `ycsb_f` / `deps_cache_overwrite`. **Não há apply nem raftlog multi-cliente.**  
   - Estender o harness: N=4 threads, barrier, cada uma corre o mesmo mix de `run_deps` (apply = 2 `batch()`, raftlog = 16 puts + get/8).  
   - `CompatEngine` já tem `batch` / `put_cf` / `get_cf`.  
   - Três runs na **mesma** caixa, mediana de ≥3:  
     `compat` (fd always) · `rocksdb` `ROCKS_PARITY_SYNC=1` · `rocksdb` `ROCKS_PARITY_SYNC=0`.  
     `ROCKS_PARITY_CLIENTS=4`.  
   - Finding: `findings/rfc0040-p11/` com as **três** colunas + p50/p95 (qps sozinho nesta caixa é ruído OrbStack).  
   - Se `compat / rocks_async` em apply ou raftlog MC **≥ 1.0** → P1.2 não mexe no group. Se **< 1.0** → P1.2 (catch-up / um `write` WAL por grupo — já existe o caminho; medir `write_group_stats` antes de chutar).

2. **RFC-0040 P2.1 — scan ≥ async** (async = sync nas leituras).  
   Residual conhecido (run5, pré-P0.3): p50 0.3 µs (cache) / **p95 22 µs vs Rocks 5 µs**; 7 SST, 2.7 sondados/scan.  
   Próximo código: menos L0 no colo do scan (worker drena até `< L0_COMPACTION_TRIGGER` depois do apply — a corrida é o que deixa L0=4); `SstCountCursor` ainda copia bounds para `Bytes` (`db.rs` `SstCountCursor::new`). Não relabel L0→L1 sem rewrite (0037 partiu o scan).

3. **RFC-0039 P0.2 — no mesmo dia da remesura.**  
   Medir `fdatasync` p50 isolado nesta caixa. Split apply: encode / mem / fd / flush. Se `2 × fd > Rocks_sync_avg / 5`, o 5× apply vs **sync** não cabe — gravar o piso, **não** relabelar o alvo. Raftlog 5× vs sync é o mais plausível (1 fd + matar cauda).

4. **RFC-0040 P2.2** só se P1.1 MC ainda perder do async: pipeline encode∥fsync no **host** (`rocksdb-compat`), nunca no core.

### Comandos

```bash
# sync peer (default do script)
ROCKS_PARITY_FULL_SYNC=0 ROCKS_PARITY_SYNC=1 ROCKS_PARITY_CLIENTS=4 \
  scripts/tikv_ycsb_parity_v0.sh findings/rfc0040-p11/sync

# async peer — o script hoje só corre rocks com SYNC=1; segunda passagem:
ROCKS_PARITY_SYNC=0 cargo run -q --release -p rocksdb-parity-bench --features real \
  --bin rocks-parity-bench -- findings/rfc0040-p11/async rocksdb

# testes antes de claim
cargo fmt -p pedradb-core -p rocksdb-compat -p rocksdb-parity-bench
cargo test --release -p pedradb-core --lib concurrent -- --test-threads=1
cargo test --release -p rocksdb-compat --test adversarial -- --test-threads=1
cargo test --release -p pedradb-core --lib count_borrowed_matches
cargo test --release -p pedradb-core --lib rfc19_change_feed_puts
git push origin HEAD:main   # todo claim vai com push
```

O script `tikv_ycsb_parity_v0.sh` **não** corre a coluna async sozinho — a próxima run tem de acrescentar o terceiro `cargo run` (ou um wrapper). `rocks-parity-compare` hoje compara um par; a tabela de 3 colunas é o finding, não o gate JSON antigo.

### Ficheiros

| o quê | onde |
|---|---|
| MC A/F/ovr (já existe) | `crates/rocksdb-parity-bench/src/lib.rs` `run_clients` |
| MC apply/raftlog (falta) | mesmo ficheiro; `run_deps` é o molde single-client |
| engine compat | `crates/rocksdb-parity-bench/src/engines.rs` `CompatEngine` |
| group / catch-up | `crates/pedradb-core/src/concurrent.rs` `WriteGroup` |
| count / SST cursor | `crates/pedradb-core/src/db.rs` `count_visible`, `SstCountCursor` |
| encode WAL | `crates/pedradb-core/src/batch.rs` `encode_ops`; `wal/mod.rs` `append_write_ops` |
| baseline quieto | `findings/tikv-ycsb-concurrent-compat/README.md` (run5, pré-P0 CPU) |

### Não fazer

- Skip `fdatasync` / fundir os 2 `write()` do apply.  
- Thread no `pedradb-core`.  
- Relabel L0→L1 sem rewrite.  
- Julgar uma run de qps (max 10–200 ms mata YCSB C). Mediana + p50/p95.  
- Vender 5× sync como “mais rápido que o Rocks”.  
- Subagent com modelo mais forte que a sessão.

### Flakes conhecidos

- `scan_prefetch_n_window_measure` (timing; passa isolado).  
- `checkpoint_during_off_lock_flush_keeps_acked` (~1 em 5).
