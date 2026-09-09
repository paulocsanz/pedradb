# 2026-09-09 — RFC-0179 compact_with_ssts_only below hard

Search: `python3 .grok/skills/caminho-sel4/scripts/candidates.py`

- unpaid_script=0 unpaid_compose=0 unpaid_concurrency=0 unpaid_scale=0
- leftover_next: compact_with_ssts_only / compact_leveled below hard
- Rank 13 skip Montanha; Rank 14 skip leftover is_empty

`compact_with` and `flush` already refuse below hard. `compact_ssts_only` /
`compact_with_ssts_only` still wrote merge SSTs (public API + auto-compact
`drain_l0_once`). Reclaim path already gated via `compact_allowed`; this
fire gates the callee itself.

cut=compact_with_ssts_only
test=failing_env_compact_ssts_only_under_hard_floor_is_disk_pressure
