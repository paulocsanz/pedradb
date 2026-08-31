# RFC-0157 P2.3 — nightly REAL TCP campaign registration 2026-08-30-r2

- date: 2026-08-31T02:49:14Z  host: Darwin arm64  K=8
- port ranges: 26000..26023 (L28_BASE_PORT seam); retries ≤3/seed
- wall K-paralelo: 130s vs 1 seed solo: 120s (fator 1,08)

| seed | attempts | s | fingerprint |
|------|----------|---|-------------|
| 0x015A_N001 | 1 | 120 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x015A_N002 | 1 | 118 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x015A_N003 | 1 | 119 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x015A_N004 | 1 | 118 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x015A_N005 | 1 | 117 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x015A_N006 | 1 | 118 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x015A_N007 | 1 | 118 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |
| 0x015A_N008 | 1 | 119 | seed=641e28 kill=node put=1 get=1 after=1 restart=1 remove=1 leave=1 left=1 hw=1 part=1 apply=1 napply=1 trunc=1 odrop=1 abort=1 nowms=1 hist=1 fence=1 clear=1 pre=1 peer=1 lid=1 rdr=1 dsc=1 pld=1 std=1 hnt=1 slot=1 dterm=1 bounded-elect not-eventual |

## Piso
- K seeds com get/after/restart/remove/napply=1 é EVIDÊNCIA, não ∀ TCP (R-swarm-real segue no residual).
- retry ≤3 é harness; liveness continua não admitida (R-es).
