# RFC-0204 P2.2 — âncora linux de barreira ISOLADA: deferido (datado)

**Data:** 2026-09-11 · **Status:** `deferido` — débito de medição datado, registrado na tabela `scripts/ratchet/host_anchors.tsv` (linha `LINUX_ISOL_DEFER_0204_P22`, valor `-`, vocabulário 0203).

## O que falta medir

O ns de `fdatasync(2)` SOZINHO numa caixa linux quiet — a barreira isolada,
distinta do pino de fases por op que já vive na tabela
(`LINUX_QUIET_0189_P01`, `enc=350;wr=890;...;grp=270` — split de telemetria
quieto do linux-p149b, 2026-09-10). O pino de fases carrega a barreira
dentro do slice `grp` por grupo confirmado; a âncora ISOLADA daria o custo
puro da barreira para comparar com `darwin_fdatasync` (17667 ns, DIAG) e
`darwin_fullfsync` (4002000 ns, DIAG) intra-classe de host.

## Por que deferido (honesto)

- O host desta sessão é macOS (aarch64) — a medição linux exigiria outra
  caixa; não há caixa linux quiet disponível agora.
- O rito existe e está fechado (RFC-0204 P2.1): o exemplo
  `fullfsync_anchor --gate-quiet` lê o loadavg 1-min ANTES de medir, recusa
  (exit 1) caixa ocupada, e numa janela quiet imprime a linha pronta
  (`row-ready`) no formato exato da tabela — em linux, para a âncora
  isolada: `linux_fdatasync  LINUX_ISOLATED_0204_P22  <ns>  <data>  <host>  quiet  <fonte>`.

## Como fechar

1. Numa caixa linux (loadavg 1-min < limiar; default 1.0):
   `cargo run -q --release -p pedradb-posix --example fullfsync_anchor -- --gate-quiet`
2. Registrar a saída completa aqui (novo finding datado) e anexar a linha
   `row-ready` na tabela com esse finding como fonte.
3. Superseder ESTA linha de deferral: coluna final
   `LINUX_ISOLATED_0204_P22 <data>` (linha nunca é apagada).
4. `cargo test -p pedradb-core --test host_anchor_table` verde (linha VIVA
   medida por classe continua única; a deferral substituída não é viva).

## Não é teorema

Ns é medição datada (RFC-0187): a tabela de âncoras nunca entra em Lean; os
multiplicadores count continuam class-independent (`WorkIo.lean` any-class).
