# diag-6b — WAL/SST `pwrite(2)` (sem `io_uring` submit_and_wait no write)

Fonte `17d10c4`. VM `linux-diag-6` **4 vCPU / 4 GB** (8 GB não coube:
`ej-backfill` ocupa o slot), Threadripper PRO 3975WX, kernel 6.12.94,
imagem `bench19diag`. 5 rounds clean `deps_raftlog`, peer `ROCKS_PARITY_SYNC=0`,
`rm` do db após cada processo.

`RESULT=BUILD_OK` 20:59:01Z, load gate 1.18, `DIAG6_DONE` 21:00:04Z.

Copied onto `main` 2026-08-25 so the launch-readiness report can cite the
number without depending on the `rfc-0054-gaps` worktree.

## Ratios

| r | pedra qps | rocks qps | ratio | p50 µs P/R | p99 µs P/R | max ms P/R |
|---|---|---|---|---|---|---|
| 1 | 87613 | 81919 | **1.070** | 10.3 / 11.5 | 32.3 / 28.5 | 0.26 / 0.29 |
| 2 | 84467 | 94697 | 0.892 | 10.4 / 10.4 | 33.1 / 20.1 | 0.27 / 0.27 |
| 3 | 84524 | 90625 | 0.933 | 10.3 / 10.1 | 33.3 / 21.1 | 0.96 / 2.52 |
| 4 | 89877 | 88370 | **1.017** | 10.0 / 10.6 | 31.4 / 19.1 | 0.23 / 2.22 |
| 5 | 87210 | 89099 | 0.979 | 10.3 / 10.4 | 33.0 / 27.6 | 0.22 / 2.32 |

**Mediana 0.979** (min 0.892, max 1.070). 2/5 rounds >1×.

Antes (uring write, bench18, 8 GB): clean r1 **0.75×**, p50 11.9 vs 10.6 µs,
p99 61 vs ~20 µs, pedra ~68 kqps.

## O que mudou

`IoUringFile::write` agora é `pwrite(2)` no cursor. `fsync`/`fdatasync`
continuam no ring (RFC-0050 CQE). O `submit_and_wait(1)` por write era
uma ida extra ao kernel em **todo** append WAL de 64 KiB — o Rocks faz
um `pwrite` só.

p50 **empatou** (10.0–10.4 vs 10.1–11.5). p99 nosso ainda ~32 µs vs
Rocks quieto ~20 µs — é o que resta no memtable/encode, não no ring.
Quando o Rocks flusha (max 2.2–2.5 ms, r3–r5) o ratio sobe; quando ele
está quieto (r2 p99 20 µs) perdemos no p99 (0.89×).

## Não é 2×

O piso de produto neste shape continua fora de alcance sem o insert do
memtable (~skip list). Este patch tira o imposto do ring, não o B-tree.
