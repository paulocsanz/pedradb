# RFC-0157 P0.3 — K-parallel REAL TCP campaign registration

- date: 2026-08-30T14:23:29Z  host: Darwin arm64  K=8
- port ranges: 26000..26023 (L28_BASE_PORT seam); retries ≤3/seed
- wall K-paralelo: 61s vs 1 seed solo: 53s (fator 1,15)

| seed | attempts | s | fingerprint |
|------|----------|---|-------------|
| 0x0157_C001 | 1 | 53 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_C002 | 1 | 50 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_C003 | 1 | 51 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_C004 | 1 | 50 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_C005 | 1 | 51 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_C006 | 1 | 50 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_C007 | 1 | 50 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |
| 0x0157_C008 | 1 | 50 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 bounded-elect not-eventual |

## Piso
- K seeds com get/after/restart/remove/napply=1 é EVIDÊNCIA, não ∀ TCP (R-swarm-real segue no residual).
- retry ≤3 é harness; liveness continua não admitida (R-es).
