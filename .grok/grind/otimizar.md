# grind / otimizar
updated: 2026-09-09T12:00:00
fire: 110
in_progress: true
consecutive_noops: 0
scheduler_id: 01a081d0d5f27db28327d48392649943

## Last fire
- started: 2026-09-08T23:55
- ended: 2026-09-09T00:10
- verdict: worked
- why: 51e0554 RFC-0180 P0.78 write_buffer_for_ram 64MiB on 4GiB
- number: ratio=0.716 pedra_qps=163935 rocks_qps=228913 shape=deps_cache_overwrite_mc4 (DIAG)
- next: ycsb_f_mc4 get_path

## This fire
- started: 2026-09-09T12:00
- ended:
- goal: engine cut that moves ycsb_f_mc4 vs Rocks SYNC=0
- forbidden: P0.72 P0.73 P0.76 grouping-knobs collapsed Darwin-as-Linux G1-1c-win kvrocks_mc50-as-win
