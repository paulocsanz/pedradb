# RFC-0176 — Modelo matemático de escala (um processo)

**Status:** done (P0–P2)
**Updated:** 2026-09-07
**ID:** 0176
**Parents:** [0153](0153-ram-scale-block-cache-bytes.md),
[0160](0160-slipstream-scale-2x.md),
[0161](0161-slipstream-scale-1p5x.md)
**Peer:** RocksDB default `sync=false`. G1 não é win.

Este RFC **é o modelo**. Não é um cronómetro. \(P\) e \(\mathrm{cap}(R)\)
são provados (Verus). \(T\) (µs) é instanciado e **medido**; \(\eta\)
(noisy neighbor) é variável de previsão, não ruído ignorado.

## Background

Duas perguntas que se misturam:

1. O `get` **olha para quantas coisas** quando n cresce? (trabalho \(P\))
2. Cada olhar **demora quanto**? (relógio \(T\))

(1) é função de n e da forma LSM. (2) depende de RAM vs disco **e** de
quem mais está na máquina. Uma reta única mente.

Constantes de produção: fanout \(F=10\), \(L_1=256\) MiB, bloco 4 KiB,
\(b=245\) B/e, bloom 10 bpk, \(\lambda\sim 10^6\) chaves/s (1 núcleo).

## Problems This Solves

- **Problem:** melhor/pior/happy misturados numa média.
- **Problem:** 3 TiB de RAM lidos como requisito do motor.
- **Problem:** noisy neighbor não estava na fórmula.
- **Problem:** previsões sem banco de medição.

## Proposed Solution

### Trabalho (provado)

\[
S = n\,b
\qquad
L = \min\{\ell : L_1\cdot 10^{\ell-1} \ge S\}
\qquad
P_{\mathrm{best}} = L+1
\qquad
P_{\mathrm{worst}} = L+4
\]

\(P_{\mathrm{as\text{-}is}} = N_{\mathrm{files}} \approx S/L_1\) é o
walk-all (bug), não o pior legal.

### Relógio (medido; \(\eta\) = noisy neighbor)

\[
T = P\cdot\bigl(H\,\tau_{\mathrm{ram}} + (1-H)\,\tau_{\mathrm{disk}}\bigr)\cdot(1+\eta)
\]

\(H, \eta\) em \([0,1]\). \(\eta=0\) caixa sozinha; \(\eta=0.5\) vizinho
come metade da banda de IO / page cache.

| caso | \(P\) | \(H\) | \(\eta\) |
|---|---|---|---|
| melhor | \(L+1\) | 1 (store cabe) | 0 |
| happy | \(L+1\) | 1 se hot, senão 0.20 | 0.10 |
| pior produção | \(L+4\) | 0 | 0.50 |
| as-is | \(N_{\mathrm{files}}\) | 0 | 0 |

Hot ⇔ \(S \le \mathrm{cap}(R)\), \(\mathrm{cap}(R)=\min(\max(3\,\mathrm{GiB},3/4 R), R-1\,\mathrm{GiB})\).
\(R_{\mathrm{hot}}\) é page cache do dataset, **não** heap do motor.

### Instâncias (happy, host 64 GiB)

| | 1B | 10B |
|---|---:|---:|
| \(S\) | 228 GiB | 2,23 TiB |
| \(P_{\mathrm{best}}/P_{\mathrm{worst}}\) | 5 / 8 | 6 / 9 |
| hot neste host? | não | não |
| \(H\) happy | 20% | 20% |
| \(T\) escala com | disco × η | disco × η |

### Calibração τ (banco de medição — sem retune)

`SCALE_TAU_RAM_NS=1100`, `SCALE_TAU_DISK_NS=13500`. Finding:
[`findings/2026-09-07-rfc0176-spectrum/`](../../findings/2026-09-07-rfc0176-spectrum/).

| célula publicada | medido | predict | ratio |
|---|---:|---:|---:|
| 50M WARM get_hit, \(P\approx 4\) | 4,3 µs | 4,4 µs (melhor) | **0,977** |
| 100M bounded get_loop/100, \(P\approx 4\) | 53,9 µs | 54,0 µs (frio, \(\eta=0\)) | **0,998** |

1B \(P=5\): melhor ~5,5 µs / happy ~61 µs / pior ~162 µs.
\(\eta=0{,}5 \Rightarrow T_{\mathrm{noisy}}/T_{\mathrm{quiet}}=1{,}5\) por construção.

`cargo test -p pedradb-core --lib scale_kernel` e o example `scale_spectrum`
**não** rodaram neste host (árvore suja: `probe_order_kernel` em falta,
`OpenOptions` fields). A calibração usa as células oficiais acima.

### Máquinas

- dentes no kernel (`scale_probes`, `scale_warm`, `scale_probes_worst`,
  `scale_predict`, `scale_happy_hot` no `catalog.json`)
- Verus: `verus/scale.rs` deletado 2026-09-09 — os 6 pares são single-artifact pagos pelo extrato Aeneas do corpo rustc (`ScaleKernel.lean` sem sorry)
- example `scale_spectrum`: quiet vs noisy (bloqueado neste host)
- Aeneas P2.1: `scripts/aeneas_scale.sh` → `SOURCE.scale`;
  `lake build Scale` verde (`point_get_probes_as_is_is_n_files` ∀).
  **Não** mede \(T\).

## Delivery slices (mandatory)

### P0 — modelo executável

- [x] **P0.1** Este RFC + `docs/status.md` — status: `done`
- [x] **P0.2** `scale_kernel.rs` \(P\) + \(\mathrm{cap}\) + 1B/10B — status: `done`
- [x] **P0.3** Twin Verus — status: `done`

### P1 — espectro + medição

- [x] **P1.1** `predict_get_ns` / `probes_worst` / `happy_hot_bps` / \(\eta\)
      + example `scale_spectrum` — status: `done`
- [x] **P1.2** `pedra scale-model` CLI — status: `done`

### P2

- [x] **P2.1** Aeneas extract + Lean `Scale.lean` (as_is ∀; não mede µs) —
      status: `done` (`lake build Scale` green; `SOURCE.scale`)
- [x] **P2.2** Escada por regime na caixa oficial — status: `done`
      (`linux-gate-p149b` 4 GiB: 10M `mode=hot` / 25M `mode=bounded-cache`;
      [findings/2026-09-07-rfc0176-p22-official](../../findings/2026-09-07-rfc0176-p22-official/README.md);
      não é DIAG local)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + status | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | kernel P + cap | done | `scale_kernel.rs` | 2026-09-07 |
| P0.3 | p0 | Verus | done | `verus/scale.rs` 10/0; 2026-09-09: mirror deletada, 6 pares aeneas-paid | 2026-09-07 |
| P1.1 | p1 | espectro + noisy + bench | done | `scale_spectrum` | 2026-09-07 |
| P1.2 | p1 | CLI | done | `pedra scale-model` → `scale_forecast` | 2026-09-07 |
| P2.1 | p2 | Aeneas extract + Lean as_is | done | `aeneas_scale.sh` / `Scale.lean` | 2026-09-07 |
| P2.2 | p2 | escada oficial | done | 10M hot / 25M bounded-cache on p149b | 2026-09-07 |

## Acceptance Criteria

- **Tests:** dentes `*_is_not_ok` para probes, warm, worst, predict, happy_hot,
  forecast. CLI `rfc0176_pedra_scale_model_prints_kernel_table` +
  `rfc0176_scale_forecast_64gib_one_and_ten_billion`. Example:
  quiet ∈ [best/4, worst×4]; noisy ≥ quiet.
- **Telemetry:** example imprime λ, η, T. Sem probe novo no engine.
- **Documentation:** este RFC; `docs/status.md`; findings
  `findings/2026-09-07-rfc0176-spectrum/` e
  `findings/2026-09-07-rfc0176-p22-official/`; `formal/aeneas/EXTRACT.md` (Scale).
- **Screenshots:** backend-only.

## Out of scope

- Run de 1B/10B. mmap. G1 como “perda”. Extrair `db.rs`.
- Afirmar \(T_{\mathrm{get}}\) O(1) em µs. Aeneas como cronómetro.
