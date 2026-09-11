# 2026-09-10 — RFC-0193 P0.5 / P1.2 / P1.3 / P2.x — meter Linux bloqueado (DIAG + blocked)

**Date:** 2026-09-10 17:38 -03
**Host gate (plano, passo 4):** guest `linux-gate-p149b` resolvível E Darwin
load1 < 8. Resultado capturado (`$S/host-gate.txt`):

- `ping linux-gate-p149b` → **cannot resolve**; `ssh -o BatchMode=yes
  linux-gate-p149b` → **Could not resolve hostname**.
- Darwin `uptime`: load averages **17,93 / 17,65 / 15,49** (gate < 8).

**Veredito: BLOQUEADO nas duas pernas.** Nenhum número Linux foi fabricado;
nada aqui é cartaz. Precedente 0190 (`2026-09-10-rfc0190-p04-meters-blocked.md`).

## Fatias fechadas por este gate

| slice | gate do RFC | fechamento |
|---|---|---|
| P0.5 meter (overwrite_mc4/mc50/apply_mc4 3 células) | Linux p149b quieto 3-run, STOP/CONT warm10, peer `sync=false` | **blocked** — reabre quando o gate passar |
| P1.2 remeter + re-pin do fixture 0192 | idem (`PEDRA_WRITE_PHASE_STATS=1`) | **blocked** — kernel re-executado localmente (vista ticket, `2026-09-10-rfc0193-telem-ticket-view.md`); fixture segue no pin quieto 0189 P0.1, rotulado |
| P1.3 perna 25M write-phase | Linux quieto | **blocked** |
| P2.1 leftover/L0 25M (0,557×) | medir primeiro | **deferred com número** (0,557× 3/3 é o dono; corte exige a caixa) |
| P2.2 prefix 100M (0,70×) | Linux 3-run primeiro | **deferred com número** |
| P2.3 ycsb_f rmw (0,766 run2; DIAG 0,530) | mecanismo + meter | **deferred com número** (dono nomeado: `template.to_vec`+intern na metade rmw) |
| P2.4 cauda GET / U-cells | Linux 3-run antes de cortar | **deferred** (anti-overfit: nenhum P0 persegue cauda 100k) |
| P2.5 Grid B (10–100×, compaction on) | Linux cartaz | **blocked** (carrega 0190 P2.2 blocked) |

## O que FOI fechado com evidência nesta árvore (não bloqueado)

- **P0.1–P0.4 done** — mecanismo + testes nomeados + gates de igualdade:
  - byte-identidade: captura pós-corte == `wal-before.bin` arquivado
    (sha256 `831d94d6…95c5d`), `$S/wal-after/`.
  - teste de ordem de dois líderes: `off_lock_write_order_survives_two_leaders`
    (env de capability falsa força intercalação real reserve/write).
  - suíte recovery/torn/reopen verde (`$S/rfc0193-recovery-suite.log`).
  - serial `-p pedradb-core --lib --test-threads=1`: 896 pass / 21 fail,
    conjunto de falhas **idêntico** ao baseline HEAD (`$S/serial-after-fix.txt`,
    diff `$S/fail-after-clean.txt` vs `$S/fail-base.txt` vazio).
  - cross-target: `cargo check -p pedradb-core --target
    x86_64-unknown-linux-musl` exit=0 (`$S/xcheck-linux-musl.txt`) — o
    caminho posix `pwrite`/`try_clone_handle` compila no alvo Linux.
- **P1.1 io_uring: veredito datado** (não aterrizado) — ver
  `2026-09-10-rfc0193-p11-iouring-verdict.md`.

## Reabrir

Quando `linux-gate-p149b` resolver (ou a caixa oficial mudar de nome) E o
Darwin load1 < 8: rodar P0.5 exatamente como o RFC pede (3 células, min-of-3,
STOP/CONT warm10, `rocks-parity-compare` peer `sync=false`,
`ROCKS_PARITY_ALLOW_SYNC_PEER` fora), re-pin do fixture 0192 no mesmo fire.
