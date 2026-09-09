# 2026-09-09 — RFC-0179 probe Err is unknown (not 0 free)

Search: `python3 .grok/skills/caminho-sel4/scripts/candidates.py`

- unpaid_script=0/17 unpaid_compose=0/17 unpaid_concurrency=0/5 unpaid_scale=0/3
- leftover_next named catch_up cursor plant — already paid
  (`catch_up_under_hard_floor_does_not_skip_cursor` in pedradb-replicate)
- Rank 13 skip Montanha; Rank 14 skip leftover is_empty factory

Live data-fate still unpaid: `Env::available_bytes` Err was inlined as
`Err(_) => None` in `admit_disk_write` / `ensure_disk_pressure_admitted`.
Kernel already admits `None`. The Err→unknown map had no named fn, no
AS-IS dente (Err as 0 free = false-refuse), no FailingEnv inject.

This fire: `disk_probe_or_unknown` + glue match + `inject_probe_err` +
named test `failing_env_probe_err_does_not_refuse_put` + Lean dual-unfold
`disk_probe_err_admits`.

cut=disk_probe_or_unknown
test=failing_env_probe_err_does_not_refuse_put
