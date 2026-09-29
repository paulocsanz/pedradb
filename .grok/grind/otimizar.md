# grind / otimizar
updated: 2026-09-29T16:10:00
fire: 111
in_progress: false
consecutive_noops: 0
scheduler_id: 01a081d0d5f27db28327d48392649943

## Last fire
- started: 2026-09-29T09:30
- ended: 2026-09-29T16:10
- verdict: worked
- why: RFC-0305 PlainBlockCache — get_hit@100M era retenção quente (knob 1 GiB inerte no caminho ponto; só ~64 MiB TLS). Cache plain compartilhado, byte-budget, 16 shards, id-por-instância, knob `set_block_cache` agora aterra no caminho ponto. A/B mesmo binário: retenção on/off = 1.85 ms vs 4.51 ms (2.44×, p=0.00). Lint 959=959 vs base 6f357fac, zero vermelho novo, allowlist intacta.
- number: ratio=0.574 pedra_qps=542 rocks_qps=944 shape=get_hit_10M (DIAG)
- next: rerun Linux cartaz 100M get_hit (caixa blocked); cold miss 528ns vs 356ns; hydrate 1.06 vs 1.38 M/s

## This fire
- started:
- ended:
- goal:
- forbidden:
