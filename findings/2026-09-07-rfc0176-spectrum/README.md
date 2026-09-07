# RFC-0176 P1 — spectrum + noisy neighbor + Aeneas

**Date:** 2026-09-07

## Clock \(T = P \cdot (H \tau_{ram} + (1-H)\tau_{disk}) \cdot (1+\eta)\)

\(\eta\) = noisy neighbor (IO/CPU/page-cache steal). Basis points in the kernel.

| caso | P | H | η |
|---|---|---|---|
| melhor | L+1 | 100% | 0 |
| happy | L+1 | 100% se hot, senão 20% | 10% |
| pior produção | L+4 | 0 | 50% (1.5×) |

## Calibração τ contra células publicadas (não um run novo de 1B)

`SCALE_TAU_RAM_NS=1100`, `SCALE_TAU_DISK_NS=13500`.

| célula | medido | predict | ratio |
|---|---:|---:|---:|
| 50M WARM get_hit 4.3 µs, P≈4 | 4300 ns | 4400 ns (best) | **0.977** |
| 100M bounded get_loop/100 53.9 µs, P≈4 | 53900 ns | 54000 ns (cold η=0) | **0.998** |

Ambos dentro da banda `[best/4, worst×4]`. τ **não** precisou de retune.

η=50% ⇒ T_noisy / T_quiet = **1.5** por construção.

## Máquinas

- Verus `verus_scale.sh`: **10 verified / 0 errors** (P + cap + worst ≤).
- Aeneas extract `scripts/aeneas_scale.sh` → `SOURCE.scale`. Lean `lake build Scale` **green** (`as_is` ∀ file-count). Clock not extracted.
- `cargo test -p pedradb-core --lib scale_kernel` **não rodou**: árvore suja alheia (`probe_order_kernel` em falta, `OpenOptions` fields). Example `scale_spectrum` existe; mesma bloqueio.

## 1B P=5 (ns)

best 5500 / happy 60610 / worst 162000.
