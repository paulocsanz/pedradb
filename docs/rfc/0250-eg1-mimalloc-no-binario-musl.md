# RFC: O binário musl do cartaz usa mimalloc

**Status:** done
**Updated:** 2026-09-23

## Background
- `:p249` (RFC-0248) deixou A4 em mediana 0,2903, A5 em 0,6241, C2 abaixo do fjall saudável. EG1 segue 62%.
- No mesmo guest, um A4 de 25M com os contadores do RFC-0249 (`:p250`, bypass, sem spin) deu **80191** qps. A soma das fases foi 4,88µs (mem 1,95, wal 1,49, admit 0,36, encode 0,40) e o `lock_wait` 10,84µs.
- Três alavancas já existentes pioraram esse número: `PEDRA_WRITE_SPIN=8000` → 51699 qps (mem 4,74); `PEDRA_ASYNC_GROUP=1` → 62283 qps, `avg_group` 1,86, espera de grupo 10,40µs; `PEDRA_WRITE_FAIR=1` → 49101 qps.
- A cópia da chave (`BatchOp::put`) aloca fora desses contadores. O binário é musl estático. O Rocks C++ não passa por esse malloc.

## Problems This Solves
- **Problem:** no cartaz musl, o malloc da libc serializa a cópia da chave e o insert do mapa, e o A4 fica em ~80k qps com o bypass.

## Proposed Solution
- O binário `rocks-parity-bench` usa mimalloc como alocador global. O motor C++ do Rocks não usa esse alocador. A onda pagante continua sem `PEDRA_WRITE_PHASE_STATS`.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Um A4 de 25M no guest com mimalloc, fase stats ligadas só nesse diagnóstico — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Onda de 3 rodadas sem phase stats: A4/A5/C2, canário ≥165000, peer `sync:false` — status: `done`

### P2 — later / polish
- [x] **P2.1** O buraco que sobra depois do mimalloc é o read lock do submit no caminho claro (RFC-0251) — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | diagnóstico mimalloc no A4 | done | :p251 | 2026-09-23 |
| P1.1 | p1 | onda Linux sem phase stats | done | :p251 | 2026-09-23 |
| P2.1 | p2 | gap fases vs 1/qps | done | RFC-0251 | 2026-09-23 |

## Acceptance Criteria
- **Tests:** o binário compila com `#[global_allocator]` mimalloc. Não há teste de qps no Darwin.
- **Telemetry / Analytics:** o diagnóstico `:p251` (phase stats ligados, uma corrida, não é cartaz) deu qps **123044**, mem 0,77µs, wal 1,34µs, lock_wait 5,55µs, contra 80191 / mem 1,95 / lock_wait 10,84 no `:p250` sem mimalloc. Não é a mediana de 3 rodadas.
- **Documentation:** este RFC e `findings/2026-09-23-rfc0250-a4-a5-c2-mimalloc-unpaid-linux/`. A onda `:p251` (3/3 canário ≥165000, sync false) deu A4 mediana 0,6225, A5 mediana 0,8986, C2 pedra mediana 317344 com fjall colapsado nas três. Nenhuma fatia fecha. EG1 segue 62%.
- **Screenshots:** backend-only.

## Out of scope
- `PEDRA_WRITE_SPIN`, `PEDRA_ASYNC_GROUP=1` e `PEDRA_WRITE_FAIR=1` como default. Os três pioraram o A4 neste guest.
- Tratar 123044 qps como vitória contra Rocks. O peer não rodou nessa corrida.
