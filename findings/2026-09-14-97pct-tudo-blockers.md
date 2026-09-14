# 97% em tudo — o que o board permite e o que o cânone/CI bloqueiam

**Date:** 2026-09-14
**HEAD at measure:** `52467c0a` then RecoveryBoot land
**Command:** `python3 scripts/sel4_gap.py` + `python3 scripts/sel4_coverage.py`

## Target vs live (two runs)

| Number | Start | Need 97% | Feasible? |
|---|---|---|---|
| DEFINING | 85.15% | ≥97% | yes — A1 unclamped 1→2 via second lake-checked refinement → **101.82%** |
| CLAIM | 75.0% | ≥97% | **no** — A9b is hardcoded 0; GitHub jobs fail with 0 steps |
| escada 0188 | 300/322 = 93.17% | 313/322 | **no** — 22 pending are 10 Montanha cartoons + 12 never-admit |
| m2 | 54/296 = 18.24% | 287/296 | **no this turn** — 233 dual-unfolds remain |
| A2b | 868/925 = 93.84% | 898/925 | not DEFINING-moving (~0.5pp) |
| A2a | 17.07% | 82% to hit DEFINING 97% with A1=1 | +107k kernel LOC; dump of db.rs forbidden |

## CLAIM / A9b — measured

`ci_github_green` is `0` in `scripts/sel4_gap.py` (not read from the tree).
CLAIM = (1 + 8/8 + 0 + 1) / 4 = 75%. The only missing term is A9b.

GitHub (repo `paulocsanz/pedradb-internal`):

- dispatch `proof-check` run **34876736452** (2026-09-14T17:46Z): conclusion failure, **5s**, every job `steps: []`
- push run **34822521806**: same empty-steps pattern
- token has `repo`/`workflow`; no billing scope. Account payments/spending limit (same class as 0222 P0.8)

Without a green GitHub run, flipping A9b to 1 is a lie. CLAIM cannot reach 97%.

## Ladder — measured

22 pending ids: Montanha `children*`/`fields*`/`pack` (10) + canon never-admit
(`forall_schedules`, `media_durable`, `lock_interleavings`, `tcg_guest`,
`fdatasync_rc`, `world_trajectory*`, `liveness_claim`, `pct_*`, `fsync_lie_tcg`,
`stacked_liars`). Registering those as atom would flip residuals the TCB
forbids (`never_floor` / `∀π` / media-durable / Montanha skip). 300/322 is the
ceiling under current canon.

## m2

54/296 chained. 97% = 287 names in `Compose*.lean` with dual-unfold rito, not
a comment list. Not a one-commit land.

## Residual

DEFINING ≥97% is the only bloco this turn can pay without rewriting
`sel4_gap.py` or violating RFC-0061. CLAIM/ladder stay blocked with the
evidence above. Not “par do seL4”.
