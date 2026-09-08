# RFC-0180 — overwrite_mc4 ≥1× vs Rocks default

**Status:** in-progress
**Updated:** 2026-09-07
**ID:** 0180
**Parents:** [0178](0178-vitoria-celulas-restantes.md) P0.11–P0.12
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
G1 não é win. Fjall absoluto, não gate.

> P0 é local (teste + DIAG Darwin isolado). 3-run na caixa 4 GiB é P1 —
> não assar sem pedido.

## Background

- Isolated `deps_cache_overwrite_mc4` (P0.11 `ONLY` + 4 clients +
  `MC_FRESH`): drop-in same-class vs Rocks default. **P0.9 p42 quiet
  3/3 mediana 1,002×** (276/213/275 k vs 275/261/267 k, `sync: false`).
  Named loss run2 0,816× (Pedra 213 k). Good Pedra ~275 k, p50 13 vs 12.
- P0.12 adaptive async group (merge se 2–8 writers): Darwin 3-run
  mediana **0,61×** (161 k vs 227–274 k). p95 107→45 µs. p50 ainda
  **20 µs vs 12 µs**.
- P0.3 idle skip: **0,63×**. P0.4 WAL on-lock: **0,47×** (reverted;
  avg_group 1,16). P0.5 spin linger: **0,66×** (190–212 k vs 297–316 k;
  p50 12 vs 11 µs; **p95 25 vs 23 µs**). 1c overwrite **0,81×**
  (309 k vs 379 k, p50 2,9 vs 2,4 µs).
- P0.7 1-op fast path **0,63×** avg_group 1,7. P0.8 linger-after-1-op
  **0,65×** avg 1,65. Two-phase linger (want 3) was noisy (host load).
- P0.12 p14 mediana **0,79×** (p50/p95 ≤ Rocks). P0.14 stage-only:
  p16 **0,81×**; FLUSHDIAG parked_n=0 — o max ~1,4 s **não é L0**.
  P0.2 sem linger: max 2,5 ms. p09-bypass: max 1,3 ms. 2048
  `spin_loop` de linger (P0.5) é o stall (OS deschedule do leader).
- O resto 1c é CPU/WAL (~0,5 µs). O resto mc4 é grupo (avg ~1,6) +
  apply serial. TLS reply no lugar de `sync_channel`. 2ª fase de linger
  revertida (p50 20 µs). Host ruidoso: 4,87× foi stall do Rocks, não win.

## Problems This Solves

- **Problem:** overwrite_mc4 publicável continua <1× no drop-in
  (0,61×). Isolado não era só a suíte.
- **Problem:** p50 20 vs 12 µs — o grupo amortiza o rabo e paga
  latência no mediano.

## Proposed Solution

Cortar o custo por op no put async 1-op a 4 clientes, sem desligar
durabilidade nem o bypass a 50 threads. Cada fatia P0 é uma hipótese
medida no mesmo harness isolado.

## Delivery slices (mandatory)

### P0 — local, até ≥1,0×

- [x] **P0.1** Este RFC — status: `done`
- [x] **P0.2** Baseline isolado: P0.12 **0,61×** 3-run — status: `done`
- [x] **P0.3** Skip idle vlog flush + idle read-cache invalidate +
      no TLS write-through on put — status: `done` (mediana 0,63×)
- [x] **P0.4** WAL `write()` on-lock no grupo async — status: `done`
      (reverted: mediana 0,47×, avg_group 1,16; barge após reply)
- [x] **P0.5** Leader linger + async catch-up (avg_group) —
      status: `done` (spin-first 0,66×)
- [x] **P0.6** Skip bulk observe on dead family — status: `done`
- [x] **P0.7** 1-op group → commit_async_one — status: `done` (0,63×)
- [x] **P0.8** Linger after 1-op when recently_multi + WAL
      encode+write one hop — status: `done` (0,65×)
- [x] **P0.9** overwrite_mc4 3-run mediana ≥1,0× vs Rocks default —
      status: `done` (p42 quiet 3/3 mediana **1,002**: 1.002 / 0.816 /
      1.032; Pedra 276/213/275 k vs Rocks 275/261/267 k; sync false/false.
      Named loss run2 Pedra 213 k p999 647 µs. p47 r3 quiet 1.004 271 vs 270 k.)
- [x] **P0.10** TLS follower reply + `settled_sst_only=false` on
      idle-cache publish — status: `done` (2nd linger reverted: p50 20 µs)
- [x] **P0.11** Sample auto-flush every 32 seqs on `commit_async_one` —
      status: `done`
- [x] **P0.12** 1-op WAL Full record + `apply_one_owned` — status: `done`
      (p14 mediana 0,79×; p50/p95 ≤ Rocks; r2 stall 0,45×)
- [x] **P0.13** No 1-op fast path when recently_multi — status: `done`
      (reverted: avg_group stuck 1,63; mediana 0,74×)
- [x] **P0.14** Async Ok flush is stage/park, never L0 write — status:
      `done` (p16 mediana 0,81×; parked_n=0 neste shape; max 1,4 s ficou)
- [x] **P0.15** Linger: 64 pause + 8 µs condvar — status: `done`
      (reverted: p17 mediana 0,50×, max 2,0–2,4 s, Darwin timer coalescing)
- [x] **P0.16** Linger off — status: `done` (p18 mediana 0,57×, max 2–6 ms,
      avg 1,35, p95 68 µs; stall gone, grouping gone)
- [x] **P0.17** Linger 2048 loads, no `spin_loop` — status: `done`
      (p19 mediana 0,52×, avg 1,23 — janela curta demais)
- [x] **P0.18** Linger 3/8 µs `Instant` busy-wait — status: `done`
      (p20 3 µs avg 1,33; p21 8 µs p50 30 µs, host noisy, avg 1,33)
- [x] **P0.19** 2048 `spin_loop` only while `active >= 2` — status: `done`
      (p22 mediana 0,60×, avg 1,9, p95 ≤ Rocks, max 0,5–0,7 s)
- [x] **P0.20** 256 `spin_loop` iff `active >= 2` — status: `done`
      (p23 mediana 0,54×, max 3–6 ms, p50 16 vs 10,5; gate perdeu o re-enter)
- [x] **P0.21** 256 `spin_loop` always (no active gate) — status: `done`
      (p24 mediana 0,60×, max 5–10 ms, p50 17 vs 12)
- [x] **P0.22** 1024 `spin_loop` — status: `done`
      (p25 mediana 0,64×, avg 2,0, p95≈Rocks, max 0,2–0,3 s)
- [x] **P0.23** 2048 `spin_loop` + lock-order — status: `done`
      (p26 mediana 0,58×, max 0,4–1,2 s; lock-order não mata o stall)
- [x] **P0.24** 512 `spin_loop` — status: `done`
      (p27 mediana 0,72×, p95≈Rocks, max 19–21 ms, avg 2,05, p999 1,4 ms)
- [x] **P0.25** Pin `commit_inflight` no 1-op e na sessão do leader —
      status: `done` (p29 mediana 0,72×; p999 1,5 ms ficou — não era barge)
- [x] **P0.26** Flush-debt observe is `try_read` — status: `done`
      (p30/p31 host noisy; p999 1,5 ms ficou)
- [x] **P0.27** `lead` pins inflight via shared Arc, no `db.read()` —
      status: `done` (p32 p999 1,5 ms ficou; stall é follower_recv)
- [x] **P0.28** Async group WAL keeps write lock — status: `done`
      (reverted: p33 p50 15,5 vs 13; P0.4 de novo)
- [x] **P0.29** Compact/flush skip lock-free when writers active —
      status: `done` (p36 mediana 1,007 vs Rocks 90–157 k — host ruidoso,
      não win; p999 Pedra 1,6 ms ficou)
- [x] **P0.30** Host worker skip *before* any Db lock — status: `done`
      (N×1-op loop reverted: p37 p95 75 µs; skip-before-lock stays)
- [x] **P0.31** `lead` one group then resign — status: `done`
      (p999 1,5 ms → 103–158 µs; quiet p39 r1 **0,91×** 237 k vs 261 k;
      p50 14,9 vs 11,5; avg 3,04. p40/p41 Rocks stalled — not a win.
      Linger 256 is dead on this path: leader always pushes own write.)
- [x] **P0.32** Skip async catch-up when batch already ≥2 at MC (≤8) —
      status: `done` (p42 quiet 3/3 mediana **1,002**: 1.002 / 0.816 /
      1.032; Pedra boa 275–276 k vs Rocks 261–275 k; p50 13 vs 12.
      Run2 Pedra 213 k p999 647 µs — named loss. avg 2.51. p43 Rocks
      stalled. p44 r1 1.036 quiet, r2 Pedra 214 k de novo.)
- [x] **P0.33** Host-worker skip hysteresis 200 µs after last Ok —
      status: `done` (p45: bad Pedra runs were `lead_write` 13–50 ms;
      L0 drain test still passes — 1 ms sleep > 200 µs)
- [x] **P0.34** Hysterese 200 µs só em `materialize_bulk`; L0-at-trigger
      e flush-debt usam inflight‖active — status: `done`
      (5–10 kQPS não empilha L0; overwrite handoff gap ainda skipa bulk)
- [x] **P0.35** Restaurar TLS last-get write-through no put (≤1024 B) —
      status: `done` (ycsb_a/f RMW; overwrite_mc4 nunca lia)
- [x] **P0.36** Leftover drain-in-lead — status: `done` (p48 avg 1,95
      e overwrite 0,87×; reverted. Canário `rfc0180_leftover_*`. Next
      leader takes WAL-late members — last-op hang residual.)
- [x] **P0.37** Catch-up skip quando batch≥2, sem gate `active≤8` —
      status: `done` (extra take + absorb ainda agrupam high-n)
- [x] **P0.38** Auto-flush size check em todo Ok 1-op — status: `done`
      (`maybe_auto_flush_with` already early-outs under limit)
- [x] **P0.39** Catch-up até 4 membros quando `2≤active≤8` (`diagnose
      write --clients 4` cut=grouping expected_group=4). n≥16 continua
      skip em 2. Teste `rfc0180_leader_linger_and_async_catchup`.
      status: `done`
- [x] **P0.40** Grupo async: encode+write num hop off Db write lock
      (`finish_group_off_lock`; `group_commit` via `group_finish`).
      G1 continua encode-under-lock then `sync_data`. Teste
      `rfc0180_async_group_wal_one_hop_recovers`. status: `done`
- [x] **P0.41** 1-op fast path só com `active < 2` (1c). A 2–8
      writers um batch de 1 membro ainda faz `group_start` (absorb +
      WAL off-lock). Não é P0.13 (`recently_multi` sticky). Teste
      `rfc0180_leader_linger_and_async_catchup`. status: `done`
- [x] **P0.42** `commit_async_ops` encode+write num hop (espelho
      grupo P0.40 / 1-op `encode_and_write_one_op`). Teste
      `rfc0180_commit_async_ops_one_hop_recovers`. status: `done`
- [x] **P0.43** Catch-up depois de `group_start` (janela do prepare:
      followers enfileiram sem o write lock). Mesmos spins que o
      catch-up pré-lock. Teste `rfc0180_leader_linger_and_async_catchup`.
      status: `done`
- [x] **P0.44** Catch-up espera `begin_submit`→queue (sem timer):
      `wait_in_flight_to_queue` enquanto `batch+queued < min(cap,active)`.
      Last-op: `active` desce sem próximo put. n≥16 cap=2. Teste
      `rfc0180_leader_linger_and_async_catchup`. status: `done`
- [x] **P0.45** O mesmo wait depois de `group_start` (janela prepare).
      Teste `rfc0180_leader_linger_and_async_catchup`. status: `done`
- [x] **P0.46** `finish_group_off_lock` espera in-flight antes do
      drain extra / WAL off-lock (async). Teste
      `rfc0180_leader_linger_and_async_catchup`. status: `done`
- [x] **P0.47** Bypass 1-op: WAL `write()` off Db write lock
      (`async_one_stage` / `async_one_publish`). 1c lone stays on-lock.
      Teste `rfc0180_bypass_async_wal_off_lock_recovers`. status: `done`
- [x] **P0.48** Bypass multi-op: WAL off lock (`async_ops_stage` /
      `async_ops_publish`). Teste
      `rfc0180_bypass_async_ops_wal_off_lock_recovers`. status: `done`
- [x] **P0.49** `queued_pending` / `active`: Release no enqueue,
      Acquire no catch-up (Darwin ARM Relaxed podia perder o join).
      Teste `rfc0180_leader_linger_and_async_catchup`. status: `done`
- [x] **P0.50** Depois do resign, o primeiro a reentrar vê `active==1`
      (`grouping_cap=1`) e fecha o grupo sozinho — WRITEPHASE avg~2.7
      vs `expected_group=4`. `wait_sibling_reentry`: 1024 `spin_loop`
      até `active` crescer, só se `recently_concurrent`. Sem condvar.
      1c não espera. Teste `rfc0180_leader_linger_and_async_catchup`.
      status: `done`
- [x] **P0.51** Primeiro arriver pós-barreira: `active==1` e
      `recently=false` → `commit_async_one` e os outros 3 agrupam
      (1+3). `wait_peer_before_lone` 256 `spin_loop` antes do lone.
      1c paga 256 pauses. Teste `rfc0180_leader_linger_and_async_catchup`.
      status: `done`
- [x] **P0.52** O wait P0.51 parava em `active>=2` (`grouping_cap=2`).
      Agora espera `expected_group=4`. 1c ainda lone depois de 256.
      Teste `rfc0180_leader_linger_and_async_catchup`. status: `done`
- [x] **P0.53** Hoped cap = `grouping_cap(last_peak)` (não magic 4).
      2-client não espera fantasmas; n≥16 leftover cap=2. `begin_submit`
      `fetch_max` o pico. Teste `rfc0180_leader_linger_and_async_catchup`.
      status: `done`

### P1 — caixa 4 GiB (pede bake)

- [ ] **P1.1** overwrite_mc4 isolado 3-run ≥1,0× na caixa — status: `todo`

### P2 — polish

- [ ] **P2.1** overwrite_mc4 3-run **3/3 quietos ≥1,0×** (não mediana
      com named loss 0,816) — status: `todo` (filho [0182](0182-same-boot-write-path-harnesses.md) P1.1)
- [x] **P2.2** Leftover hang — status: `done` (filho [0181](0181-leftover-follower-nao-pende.md) P0.3 steal)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | Baseline isolado | done | P0.12 0.61× 3-run | 2026-09-07 |
| P0.3 | p0 | Idle vlog/cache skip | done | 0.63× 3-run | 2026-09-07 |
| P0.4 | p0 | WAL on-lock (reverted) | done | 0.47× avg_group 1.16 | 2026-09-07 |
| P0.5 | p0 | Leader linger + async catch-up | done | spin-first 0.66× p95≈Rocks | 2026-09-07 |
| P0.6 | p0 | Skip bulk observe when dead | done | 1c 0.81×; `bulk_observe_needed` | 2026-09-07 |
| P0.7 | p0 | 1-op group → commit_async_one | done | 0.63× `async_one_op_fast_path` | 2026-09-07 |
| P0.8 | p0 | Linger after 1-op + WAL one hop | done | 0.65× avg_group 1.65 | 2026-09-07 |
| P0.9 | p0 | overwrite_mc4 ≥1× 3-run | done | p42 quiet median 1.002 (named loss 0.816) | 2026-09-07 |
| P0.10 | p0 | TLS follower reply + settled fix | done | 2nd linger reverted | 2026-09-07 |
| P0.11 | p0 | Sample auto-flush every 32 seqs | done | `async_flush_check_due` | 2026-09-07 |
| P0.12 | p0 | 1-op WAL Full + apply_one_owned | done | p14 0.79× p50 beat Rocks | 2026-09-07 |
| P0.13 | p0 | No 1-op fast path when multi | done | reverted: avg_group 1.63 | 2026-09-07 |
| P0.14 | p0 | Async Ok stage-only flush | done | p16 0.81×; stall not L0 | 2026-09-07 |
| P0.15 | p0 | Linger 64+8µs condvar | done | reverted: p17 0.50× max 2.3s | 2026-09-07 |
| P0.16 | p0 | Linger off | done | p18 0.57× max 2–6ms avg 1.35 | 2026-09-07 |
| P0.17 | p0 | Linger 2048 loads, no spin_loop | done | p19 0.52× avg 1.23 | 2026-09-07 |
| P0.18 | p0 | Linger 3/8µs Instant | done | p20/p21 avg 1.33; 8µs p50 30 | 2026-09-07 |
| P0.19 | p0 | 2048 spin_loop iff active>=2 | done | p22 0.60× avg 1.9 max 0.5s | 2026-09-07 |
| P0.20 | p0 | 256 spin_loop iff active>=2 | done | p23 0.54× max 3ms p50 16 | 2026-09-07 |
| P0.21 | p0 | 256 spin_loop no active gate | done | p24 0.60× max 5ms p50 17 | 2026-09-07 |
| P0.22 | p0 | 1024 spin_loop | done | p25 0.64× avg 2.0 max 0.2s | 2026-09-07 |
| P0.23 | p0 | 2048 spin + lock-order, no gate | done | p26 0.58× max 0.4–1.2s | 2026-09-07 |
| P0.24 | p0 | 512 spin_loop | done | p27 0.72× p95≈Rocks max 20ms | 2026-09-07 |
| P0.25 | p0 | inflight pin 1-op + lead session | done | p29 0.72×; p999 1.5ms not barge | 2026-09-07 |
| P0.26 | p0 | flush-debt try_read | done | p30/p31 p999 stayed 1.5ms | 2026-09-07 |
| P0.27 | p0 | lead pin lock-free Arc | done | p32 p999 1.5ms; stall=follower_recv | 2026-09-07 |
| P0.28 | p0 | async WAL on-lock | done | reverted p33 p50 15.5 | 2026-09-07 |
| P0.29 | p0 | compact/flush skip lock-free | done | p36 1.007 vs stalled Rocks — not a win | 2026-09-07 |
| P0.30 | p0 | skip before lock (N×1-op reverted) | done | p37 p95 75µs; skip stays | 2026-09-07 |
| P0.31 | p0 | lead one group then resign | done | p39 r1 0.91 quiet; p999 103µs avg 3.04 | 2026-09-07 |
| P0.32 | p0 | skip catch-up when already grouped at MC | done | p42 quiet median 1.002; run2 Pedra 213k named loss | 2026-09-07 |
| P0.33 | p0 | host skip 200µs hysteresis | done | p45 lead_write; L0 drain test still ok | 2026-09-07 |
| P0.34 | p0 | hysteresis only on materialize_bulk | done | L0/flush stay inflight‖active | 2026-09-07 |
| P0.35 | p0 | restore TLS put write-through | done | ycsb_a/f RMW | 2026-09-07 |
| P0.36 | p0 | leftover drain-in-lead | done | p48 0.87× avg 1.95 — reverted; canary stays | 2026-09-07 |
| P0.37 | p0 | catch-up skip batch≥2 any n | done | no active≤8 gate | 2026-09-07 |
| P0.38 | p0 | flush size check every async Ok | done | no 31-op overshoot | 2026-09-07 |
| P0.39 | p0 | catch-up to 4 when 2–8 writers | done | grouping lever; n≥16 still skip at 2 | 2026-09-08 |
| P0.40 | p0 | async group WAL one hop off lock | done | ConcurrentDb finish_group_off_lock encode_and_write_op_batches | 2026-09-08 |
| P0.41 | p0 | 1-op fast path only when active<2 | done | MC 1-member stays on group_start; 1c unchanged | 2026-09-08 |
| P0.42 | p0 | commit_async_ops WAL one hop | done | encode_and_write_op_batches; G1 lone still encode-then-fd | 2026-09-08 |
| P0.43 | p0 | catch-up after group_start prepare | done | same spins as pre-lock; absorb then off-lock WAL | 2026-09-08 |
| P0.44 | p0 | wait in-flight begin_submit→queue | done | no timer; stop on cap or active drop | 2026-09-08 |
| P0.45 | p0 | wait in-flight after group_start | done | same helper; prepare window | 2026-09-08 |
| P0.46 | p0 | wait in-flight before off-lock WAL | done | finish_group_off_lock extra drain | 2026-09-08 |
| P0.47 | p0 | bypass 1-op WAL off Db write lock | done | async_one_stage/publish; 1c on-lock stays | 2026-09-08 |
| P0.48 | p0 | bypass multi-op WAL off lock | done | async_ops_stage/publish; commit_async_ops uses same stage | 2026-09-08 |
| P0.49 | p0 | queued_pending Release/Acquire | done | Darwin ARM catch-up sees enqueue | 2026-09-08 |
| P0.50 | p0 | wait sibling re-entry after resign | done | active==1 hole; 1024 spins; 1c skips | 2026-09-08 |
| P0.51 | p0 | wait peer before lone (barrier start) | done | 256 spins; 1c still lones | 2026-09-08 |
| P0.52 | p0 | wait_peer target expected_group=4 | done | not stop at 2; 1c still lones | 2026-09-08 |
| P0.53 | p0 | hoped cap = grouping_cap(last_peak) | done | 2-client / n≥16 leftover | 2026-09-08 |
| P1.1 | p1 | 3-run caixa | todo | — | 2026-09-07 |
| P2.1 | p2 | 3/3 quiet ≥1× | todo | 0182 P1.1 | 2026-09-07 |
| P2.2 | p2 | leftover hang | done | 0181 P0.3 steal | 2026-09-07 |

## Acceptance Criteria

- **Tests:** unit test that drives the shipped policy/path (same style
  as `rfc0178_async_merge_adaptive_small_n_only`).
- **Telemetry / Analytics:** isolated overwrite_mc4 JSON + compare;
  peer `"sync": false`.
- **Documentation:** this RFC; 0178 keeps the inventory row.
- **Screenshots:** backend-only.

## Out of scope

- G1 write row as win. Peer `sync=true`. Fjall as gate.
- Bake 4 GiB box. mmap, `unsafe`, chunk 4 MiB, RFC-0175.
- ycsb_f_mc4 3/3 on the box (0182 / 0178 P1.4). Apply skiplist (0183).
