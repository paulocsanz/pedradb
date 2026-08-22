# P1.3 re-arm battery at the TLS-cache fix (2026-08-22, commit 312e354)

Full P0.4-method battery (3× g1/async/kvr at n=2000 + 2M long window, peer
RocksDB default `sync=false` verified in every compare) run from a clean
worktree at `312e354` — the TLS last-get cache redesign
(`findings/2026-08-22-p13-get-tls-cache/`).

## Status: supporting evidence, NOT the official record

The quiet gate passed (load 8.85 at fire, 9.27 for round-1 compat legs) but
box load rose to **12–15 for the rest of the battery** (per-leg trace in
`loads.txt`). The P0.4 standard (quiet <10 throughout) was violated mid-run,
so this battery is demoted to evidence; the official closing battery is
re-armed under the same script. Same treatment as P0.4's v2 (wrong
conditions → demoted, decomposition kept).

## Results (median-of-3 unless noted)

| column | shape | this battery | P0.4 clean (quiet) | reading |
|---|---|---|---|---|
| kvr | **kvrocks_get** | **5.98/6.28/6.46 — 3/3 ≥5** | 3.23 med | fix: compat 7.6–7.8M → **14.2–15.2M** (+90%), peer stable 2.3–2.4M |
| kvr | get long 2M | **7.04** | 4.71 | compat 11.8M → 16.0M; peer 2.49M → 2.27M (±5%, load-insensitive) |
| kvr | set long 2M | 4.65 | 5.45 (3/3) | compat 1.97M → 1.61M at load 14 — load signature; quiet re-run pending |
| kvr | pipeline | 2.33/5.34/5.31 | 5.04 med; long 5.38 | r1 compat 105k vs r2/r3 235k/224k — one anomalous leg; long exactly 5.00 |
| async | ycsb_f | **3.33 med** (2.93–3.37) | 1.40 | fix: compat 0.78M → 1.54–1.80M (+100%), peer ~0.53M stable |
| async | ycsb_a | 3.65 med (4.54/0.94/3.65) | 2.98 | compat 1.9M → 2.4–3.0M (+25–57%); r2 0.61M is a load-spike casualty |
| async | ycsb_e | 4.94 med | 5.88 (3/3) | compat 1.35–1.39M → 1.10–1.20M (−15%) at load 14; peer 0.23M stable both |
| async | ycsb_c | 3.95 med | 4.83 | compat 10.7–10.9M → 8.9–9.1M (−17%); pure reads — load, not the fix (controlled A/B at HEAD: c **+57%**) |
| g1 | c / e | 3.85 / 1.81 | 4.67 / 2.17 | same −15–20% load signature |

## Attribution method

Rocks peer absolute qps is stable across both batteries and across load
9→14 (rounds get 2.33–2.46M then vs 2.35–2.41M now; long 2.49M vs 2.27M;
async a/e/c peers within ±5%) — so every ratio movement decomposes into its
compat-side absolute. Three signatures:

1. **Fix gains** (get, f, a): compat up 25–100% with peer flat — matches the
   controlled same-box A/B at HEAD (`+43%` GET 200M, `+51/57/61%` a/c/f).
2. **Load losses** (e, c, set-long, g1 column): compat down 15–20% with peer
   flat — shapes the fix does not touch (scans, pure writes), all softened
   by the same factor while load sat at 12–15.
3. **One-off anomalies** (pipeline r1 105k, a r2 0.61M): single legs, not
   reproduced in adjacent rounds.

## Why GET is safe to trust even from this battery

Conservative composition from controlled numbers only: P0.4 quiet long compat
11.80M × the A/B fix factor measured back-to-back at HEAD (11.50M → 14.91M on
the identical 2M window) = 15.3M; against the P0.4 quiet peer (2.49M) that is
**≥ 6.1×**. The battery's own 3/3 ≥5 rounds and 7.04 long agree. Official
closure still waits for the quiet battery per house standard.
