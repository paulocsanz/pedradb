# nightly 2026-08-31-r0-dirty — distinct-world campaign on UNCOMMITTED co-agent WIP (user-authorized; attribution by content hash, see dirty_attribution.txt)

- date: 2026-08-31T20:23:35Z  host: Darwin arm64  K=8
- port ranges: 26000..26023 (L28_BASE_PORT seam); retries ≤3/seed
- wall K-paralelo: 127s vs 1 seed solo: 118s (fator 1,08)

| seed | attempts | s | fingerprint |
|------|----------|---|-------------|
| 0x15c01 | 1 | 118 | seed=15c01 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x15c02 | 1 | 116 | seed=15c02 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x15c03 | 1 | 118 | seed=15c03 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x15c04 | 1 | 117 | seed=15c04 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x15c05 | 1 | 118 | seed=15c05 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x15c06 | 1 | 117 | seed=15c06 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x15c07 | 1 | 118 | seed=15c07 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x15c08 | 1 | 116 | seed=15c08 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |

## Piso
- K seeds com get/after/restart/remove/napply=1 é EVIDÊNCIA, não ∀ TCP (R-swarm-real segue no residual).
- retry ≤3 é harness; liveness continua não admitida (R-es).

## CORRIDA EM ÁRVORE SUJA — limitações (2026-08-31)

Autorizada explicitamente pelo usuário enquanto o WIP dos kernels do
co-agente seguia sem aterrissar. Este registro **não é atribuído a um
commit**: descreve conteúdo de ficheiros não-commitados.

- Atribuição: `dirty_attribution.txt` (HEAD `eced146` + sha256 de cada
  ficheiro sujo) + `dirty_tree.patch`, fotografados às 20:18:55Z.
- Binários compilados às 20:19:30Z (35s após a fotografia). Na fotografia,
  `db.rs` e `lib.rs` estavam limpos (= HEAD); ambos ficaram sujos e
  `sst/table.rs` mudou de hash DEPOIS — o build dos testes às ~20:23 já
  falhava com 5 erros de compilação em `db.rs` (E0382 etc.), ao passo que
  o build da campanha aos 20:19:30 passou. Ambiguidade residual: se o
  edit de `db.rs` começou dentro da janela de 35s com um intermediário
  compilável, o binário difere da fotografia nesse ficheiro.
- Guardas PCT (`planted_chain3_found_by_pct_d3`,
  `planted_chain3_pct_d4_nightly`): **não correram** — `cargo test -p
  pedradb-world` não compila nesta árvore (erros do WIP acima). Os
  ficheiros `pct_d3.txt`/`pct_d4.txt` registam o compile error, não uma
  falha dos guardas. Ficam pendentes para o registro limpo r1.
- Seeds `0x15c01..0x15c08` são exclusivas desta corrida; o registro
  limpo usa `0x15b01..0x15b08` (nenhuma reutilização).
- Primeira campanha da história do programa com 8 mundos de fato
  distintos (verificação por eco de seed em cada fingerprint;
  `findings/2026-08-31-campaign-seed-collapse/`). Verde em todos os
  bits numa árvore suja **não abona nada** — o registro oficial é o r1
  em árvore limpa.
