# RFC: A mmap do WAL é uma janela, não o log inteiro

**Status:** done (engineering win, no ladder pay)
**Updated:** 2026-09-23

## Background
- `:p253` (RFC-0252, uma cópia do frame) não pagou A4. Três rodadas válidas, canário 176527 / 213051 / 214084, peer `sync: false`. A4 mediana **0,6176** (0,7785 / 0,6176 / 0,5879). A5 mediana **1,0664**, mínimo **0,9206** — a fatia pede 3/3 ≥1,0, e a r3 ficou em 0,9206. C2 com fjall em 153–161k, colapsado. EG1 segue 62%.
- No mesmo binário, um A4 curto (20k registros, não é cartaz) deu **257785** qps, acima do Rocks de ~232k da onda. O A4 de 25M fica em 149–181k.
- A mmap do WAL cobre o arquivo inteiro a partir do offset 0 e, a cada 64 MiB, desmapeia e remapeia o log todo. Depois do seed de 25M o log tem vários GiB.

## Problems This Solves
- **Problem:** cada avanço da mmap recria uma VMA do tamanho do log inteiro, e o overwrite de 25M paga isso. O mesmo put num log pequeno não paga.

## Proposed Solution
- A mmap do append é uma janela de 64 MiB alinhada no ponto de escrita. Um frame que cruza a borda alonga essa janela. O append seguinte desliza. O arquivo em si não encolhe. A recuperação continua lendo o arquivo, não essa janela.

## Result (`:p254`, 3 rodadas)
- r1 canário 162046 inválida; r2 224153 / r3 287713 válidas, peer `sync:false`.
- A4 mediana válida **0,7038** (0,6087 / 0,7982) — dentro do ruído do
  0,6176–0,7175 das ondas `:p253`/`:p252`. A hipótese do remap inteiro
  não era o gargalo: o qps mede as 100k ops, não a semente que pagava
  os remaps.
- A5 0,9401 / 0,6617. C2 pedra mediana 366966, fjall colapsado
  (154–163k). Nenhuma fatia paga.
- O log do A4 trava o próximo dono: `avg_group=1.00, queued=0` — os
  4 escritores committam sozinhos no bypass (regra 0201:
  writers ≤ ncpu), e o custo 20k→25M aparece como cauda de latência
  dentro da lock (p50 10,2 µs, p95 76,1 µs, máx 4,1 ms).

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Puts que passam de 64 MiB reabrem com a primeira e a última chave, e a maioria dos frames continua na memcpy — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Onda Linux de 3 rodadas, sem phase stats, A4/A5/C2, canário ≥165000, peer `sync:false` — status: `done` (`:p254`, impago;
  `findings/2026-09-23-rfc0253-a4-a5-c2-wal-window-unpaid-linux/`)

### P2 — later / polish
- [x] **P2.1** Se o mínimo do A5 continuar abaixo de 1,0 ou o A4 continuar abaixo de 1,0, o próximo corte é o que o phase split dessa imagem mostrar — status: `done` (donos nomeados:
  o commit solo dos 4 escritores e a cauda de latência no 25M; ver
  TRAJETORIA v9)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | janela de 64 MiB reabre | done | RFC-0253 | 2026-09-23 |
| P1.1 | p1 | onda Linux A4/A5/C2 | done | :p254 impago | 2026-09-23 |
| P2.1 | p2 | dono que sobrar | done | diagnóstico :p254 | 2026-09-23 |

## Acceptance Criteria
- **Tests:** `rfc0253_wal_window_slides_past_64mib_and_reopens` escreve ~80 MiB, exige pelo menos dois mapas, pelo menos 15000 cópias mmap em 20000 puts, e relê a primeira e a última chave. `rfc0242_small_puts_skip_per_op_pwrite_and_reopen` continua sem um `pwrite` por put.
- **Telemetry / Analytics:** nenhuma na onda pagante.
- **Documentation:** este RFC. O percentual EG1 não muda até uma fatia pagar.
- **Screenshots:** backend-only.

## Out of scope
- Marcar A5 como pago. A mediana é 1,0664 e o mínimo é 0,9206. A escada pede 3/3 ≥1,0.
- Soltar o write lock durante a cópia.
- Deixar de fdatasyncar antes do Ok no produto.
