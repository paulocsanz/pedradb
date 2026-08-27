# diag-6 — stall probe no Linux (Railway metal, não o gate caixote)

Copied onto `main` 2026-08-25 (source: worktree `rfc-0054-gaps`).

Caixote brasil recusou provisionar (saldo −R$242, `all containers failed
before first start`; GHA billing também). Workaround: Railway metal
builder, kernel 6.18.15 x86_64, **AMD EPYC 9655, 48 vCPU, 322 GB RAM**.
Isso **não** é a VM 4 vCPU do diag-5 — serve para nvcsw/nivcsw + A/B
io_uring/posix em Linux amd64 real. Projeto `pedradb-diag6` apagado
depois dos logs.

Imagem buildada de `rfc-0054-gaps` @ `7599d36` (`/proc/thread-self`).
Deploy `afadc9c2-5169-4f9f-8d5f-c124181bf8f1`. `RESULT=DIAG6_DONE`
2026-08-24T19:32:35Z.

**Nota:** esta bateria é **pré-`pwrite`**. O write path ainda era
`io_uring submit_and_wait(1)` por append. A remesura pós-`pwrite`
(fonte `17d10c4`, 4 vCPU) está em
[`../2026-08-24-linux-diag6-pwrite/`](../2026-08-24-linux-diag6-pwrite/README.md).

## Ratios (`deps_raftlog`, peer `ROCKS_PARITY_SYNC=0`)

| round | probe-uring | probe-posix | clean |
|---|---|---|---|
| r1 | 1.025 | 1.010 | 0.809 |
| r2 | 0.812 | 0.836 | 0.755 |
| r3 | 0.874 | 0.793 | **1.023** |
| mediana | 0.874 | 0.836 | **0.809** |

`uring ≈ posix` em todos os rounds → o `submit_and_wait(1)` do ring
**não** é o gap neste metal. Probe qps ~2× menor que clean porque o
`/proc` + snapshot de fase estava *dentro* do timer de build.

## Stall (probe, 4000 ops/round)

Todas as 9 pernas probe (3 uring + 3 posix + 3 rocks):

```
slow(>1ms)=0  stall_total=0.0ms  nvcsw=0  nivcsw=0
```

Pedra probe p50: batch **5.8µs**. Fases/commit: prepare 0.5µs wal 1.2µs
mem 2.5µs publish 1.0µs flsh 0.03µs lock_wait 0.

Clean (sem probe): p50 build 0.7–0.8µs, batch 5.8–6.0µs, max 0.26 ms —
sem cauda de 100 ms.

## Leitura

No metal quieto 48 núcleos o ~0.80× do clean é **p50 de commit**
(mem 2.5µs + wal 1.2µs), não stall / não 4 vCPU / não Intel-vs-AMD.
H4-flush e H-uring eliminadas neste box. A remesura que falta para
“>1× sempre” é a mesma bateria **depois** do `pwrite` no write path.
