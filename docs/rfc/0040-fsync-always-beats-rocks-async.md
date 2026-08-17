# RFC-0040: `fdatasync` always-on mais rápido que Rocks **async**

**Status:** in-progress  
**Updated:** 2026-08-17  
**Parents:** [0039](0039-apply-raftlog-scan-5x-rocks-sync.md) (5× vs sync), [0037](0037-apply-off-put-and-2x-pedra.md) (group commit + host worker), [0036](0036-tikv-rocks-2x-fdatasync.md) (G1 = `fdatasync` antes do Ok)

## Background

- Pedra no Ok **sempre** `fdatasync` (G1). Rocks default (`sync=false`) não. Isso **não** é um teto: o piso é por **ack/grupo**, não por chave.
- Arquitetura que paga um fd e mesmo assim passa o async: group commit (N ops / 1 fd), pipeline (encodar o próximo grupo enquanto o líder sinca), **uma** cópia do payload até o WAL, compact/flush fora do Ok, scan sem setup de 20 µs.
- Hoje o write path copia o payload **duas vezes** além do inevitável (encode lógico → `Vec`, depois `WalWriter` junta header+payload noutro `Vec`) e o `apply_record` clona `Bytes` outra vez para a mem. O `count` abre cada SST com `Box<dyn FnMut>` + bounds em `Bytes`.
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

- [ ] **P1.1** Harness MC (N=4) apply + raftlog vs Rocks **async** e vs sync; publicar as duas razões — status: `todo`
- [ ] **P1.2** Se P1.1 apply_async < 1.0: janela de group já medida; só então mexer no catch-up / um `write` WAL — status: `todo`

### P2 — scan vs async + pipeline

- [ ] **P2.1** `deps_scan` ≥ Rocks async da mesma run (async=sync nas leituras) — status: `todo`
- [ ] **P2.2** Pipeline encode∥fsync no **host** se P1.1 ainda perder no MC — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este doc | 2026-08-17 |
| P0.2 | p0 | encode_into + scratch + move-to-mem | done | este commit | 2026-08-17 |
| P0.3 | p0 | count cursor sem Box / sem clone | done | este commit | 2026-08-17 |
| P1.1 | p1 | MC vs async+sync | todo | — | 2026-08-17 |
| P1.2 | p1 | group se P1.1 < 1 | todo | — | 2026-08-17 |
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
