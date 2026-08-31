# RFC-0067 — âncora REAL do caminho lab `RpcMode::Direct` (R-direct-rpc)

**Date:** 2026-08-30 · **Host:** Darwin arm64 (esta máquina) · **Não** é World/sim.

`real=` na linha R-direct-rpc = execução real (host real, `StoreCluster` real em
dirs reais com fsync real) do caminho lab publicado na residual, mais as guardas
fail-closed que mantêm produção em `Queued`.

## O que rodou (stdout.txt = consoles literais)

- **38/38** testes da suíte `layers` (`cargo test -p pedradb-store --lib layers::`,
  29,4s), todos abrindo via `StoreCluster::open_lab_direct` — o caminho nomeado na
  residual (`enable_lab_direct_rpc` → `set_rpc_mode(Direct)`). Eleições
  (`elect_all`) e `PeerMsg` (RequestVote/AppendEntries) pelo pump síncrono
  in-process (0067 P2.2).
- **4/4 guardas fail-closed:** `default_open_starts_queued` (produção abre
  Queued), `pin_dst_queued_refuses_direct_switch`,
  `open_single_node_refuses_direct_switch` (pedradb-store);
  `world_attempt_direct_after_pin_stays_queued` (pedradb-world).

## O que isto NÃO é

- Não é multi-processo TCP — isso é R-swarm-real (`cluster_real`). Direct é
  in-process por construção: mesmo `PeerMsg`, pump síncrono.
- Não fecha a residual: produção continua `Queued`; Direct segue lab opt-in;
  campanha não é teorema de que todo RPC é TCP. A linha continua `continuous`.
