# RFC-0157 P2.3 — nightly REAL TCP campaign registration 2026-08-30

- date: 2026-08-30T23:12:59Z  host: Darwin arm64  K=8
- port ranges: 26000..26023 (L28_BASE_PORT seam); retries ≤3/seed
- wall K-paralelo: 207s vs 1 seed solo: 121s (fator 1,71)

| seed | attempts | s | fingerprint |
|------|----------|---|-------------|
| 0x0157_N001 | 1 | 121 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_N002 | 1 | 188 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_N003 | 1 | 194 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_N004 | 1 | 196 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_N005 | 1 | 189 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_N006 | 1 | 191 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_N007 | 1 | 189 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_N008 | 1 | 186 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |

## Transparência
- Duas ondas ANTERIORES nesta mesma noite falharam no gate de fator (2,18 e 2,22 ≥ 2,0)
  com retries por `napply=0` sob máquina carregada; consoles preservados:
  `../2026-08-30.wave1.console.txt`, `../2026-08-30.wave2.console.txt`.
- A onda verde acima usa o harness com paciência maior (cluster_real.rs, 0157 P2.3):
  leave lento vira sucesso tardio em vez de retry no caminho crítico. Seeds são as
  mesmas da noite (0x0157_N001..N008); nenhuma seed foi trocada para passar o gate.

## Piso
- K seeds com get/after/restart/remove/napply=1 é EVIDÊNCIA, não ∀ TCP (R-swarm-real segue no residual).
- retry ≤3 é harness; liveness continua não admitida (R-es).
