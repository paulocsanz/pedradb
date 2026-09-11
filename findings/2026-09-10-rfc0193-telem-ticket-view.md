# 2026-09-10 — RFC-0193 telemetria pós-P0: vista ticket do kernel 0192

**Pedido:** "refinar nossa telemetria e previsão determinística".
**Contexto:** 0193 P0.1–P0.4 aterrados (frame off-lock + ticket + `pwrite`
posicional quando o env declara capability; fallback in-lock idêntico a hoje).
O kernel 0192 preveria um pós-corte que não sabia nomear: com `wr` fora do
`wal.lock()`, qual é a CS restante e quem é o próximo dono?

## O que aterrizou (kernel `write_cycle_kernel.rs`)

- `serial_cs_ticket_ns = serial_cs_ns − wal_write − wal_encode` — a CS
  prevista depois do corte (encode e write off-lock; sobra a reserva de
  ticket, mínima por construção).
- `name_cut_ticket` — o próximo dono estrutural na vista pós-P0.
- `predicted_qps_ticket` / `ticket_cs_ns` / `ticket_qps_hat` /
  `ticket_cut` em `WriteCycleForecast`, impressos pelo `render()` como
  última linha e pelo `pedra scale-model write` (mesmo kernel, zero I/O).
- WRITEPHASE pós-P0.4 no caminho board: `wal_encode_ns` = encode off-lock,
  `wal_write_ns` = write + `wait_contiguous`, `wal_lock_hold_ns` =
  aquisição→release (walk+reserva no board path). Sem o env de stats,
  zero `Instant` no caminho quente (latch único `PEDRA_WRITE_PHASE_STATS`).

Testes: `rfc0193_ticket_view_drops_encode_and_write` (vista = cs − wr − enc,
cut re-nomeia), `rfc0192_forecast_is_the_cli_table` e
`concurrent::tests::rfc0192_write_cycle_line_uses_kernel` (inalterados,
verdes ×2).

## Número (CLI, duas execuções idênticas — `$S/write-forecast.txt`)

```
pedra scale-model write --fixture linux-quiet --leaders 4 --guard 0

cut=wal_write cut_as_is=lock_hold
cs_ns=3290 cs_as_is_ns=2200
lock_wait_hat_ns=2467 lock_wait_as_is_ns=500
cycle_ns=5750 qps_hat=173913 off_wr_qps_hat=238095
ticket_cut=publish ticket_cs_ns=2050 ticket_qps_hat=278784
```

- Vista ticket = **+60,2%** sobre o qps_hat as-is pós-0190 (278 784 vs
  173 913) no L=4 do pin — o corte 0193 no teto do modelo.
- `ticket_cut=publish` (820 ns) — ver ressalva do pin abaixo.

## Ressalva honesta do pin (não escondida)

- O fixture `LINUX_QUIET_0189_P01` continua sendo o **último split quieto
  medido** (2026-09-10, pre-0193). **Não foi re-pinado**: a perna Linux
  quieta está bloqueada (guest `linux-gate-p149b` não resolve; Darwin
  load1 17,9 — `2026-09-10-rfc0193-p05-meter-blocked.md`).
- `publish=820` no pin é o número **pre-0189-P0.3**; o seqlock-skip só
  zera os 2 RMWs em shapes só-escrita (`fills == 0`). Em shapes com
  leitura (apply_mc4, ycsb_f, ycsb_c) o publish paga os 2 RMWs — o 820
  segue vivo lá. `ticket_cut=publish` é modelo-sobre-pin-velho: o próximo
  dono REAL decide-se no re-split quieto, não aqui.
- Erro do modelo vs medido pós-P0: **não computável agora** (meter
  bloqueado). Último ponto de dados: pre-0190 o hat foi pessimista
  (off 123 426 < medido 125 313) — o teto do modelo não é promessa.

## Reabrir

Re-split Linux quieto pós-0193 (`PEDRA_WRITE_PHASE_STATS=1`, STOP/CONT
warm10) re-pina o fixture e promove `ticket_cut` de modelo a atribuição.

## Correção datada (posterior — 0192 P0.4)

O "+60,2%" acima é **aritmética teto-contra-teto** e não previsão de ganho:
`278 784` era `1e9/ciclo` de um pipeline serial determinístico, não o QPS
de L clientes. Na distribuição medida r2 o mesmo corte de 1 240 ns projeta
**+33‰** (103 626 → 107 066) — ver
`findings/2026-09-10-write-forecast-why-it-missed.md` e o tier calibrado
do kernel (`tier=forecast`).
