# RFC-0191 P2.3 (passo 12) — lease graduado a atom (cap 119→118)

**Data:** 2026-09-10 · **Fire:** 770 · **Estado:** landed

## Alvo

O par `lease` (F7/F56, `pedradb-dcs/src/lease_kernel.rs`, entry
`lease_live`) estava parado em `data_fate`. A garantia: **uma lease
está viva exatamente quando é zero (nunca expira) ou o relógio está
abaixo do expiry** — lease expirada nunca está viva. Primeira graduação
no crate dcs (Lease.lean).

## Teorema (atom registrado)

`lease_live_iff_zero_or_now_below` (`Lease.lean`, ∀ `lease now_ms`):
`ok true` ↔ (`lease = 0#u64` ∨ `now_ms < lease`). 0 sorry, verde na
primeira build. Padrão dos fires 767–768: disjunção de equação + comparação
Prop nua; `simp at h` reduz `ok (decide (now < lease)) = ok true` até o
Prop; backward `split` + `simp [hlt]`.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 119 | **118** (`lease` gradua) |
| `floor_atom` | 12 | **13** |
| `floor_extract` / residuals `extract` | 267 | **266** (recount) |
| residuals `proof_depth` | 267/2/12/17 | **266/2/13/17** |
| residuals `data_fate`/`single_artifact` | 119/291 | **118/291** |

## Evidência

- `lake build Lease` → verde, 0 sorry.
- `cargo test -p pedradb-dcs --lib lease_live_on_live_expiry` →
  planta (no próprio kernel) verde.
- depth-floor GREEN (extract 266, atom 13/13, data_fate 118≤118);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.

## Fila de pagamentos (varredura de 119 data_fate restantes)

19 com if/match real no extrato: `ae_ack`, `ae_entry`, `blob_gc_pick`,
`cf_encode_effective`, `compact_decision`, `compact_retention`,
`compact_rewrites_sst_cf`, `compact_unleft` (morto: iff FALSE),
`decode_cf_key`, `encode_cf_key`, `infer_sst_cf`, `lease` (este),
`leveling`, `manifest_recover`, `pin_gc`, `reopen_outcome`,
`sched_plant_joint`, `unreserve_si_gen` (morto: iff FALSE),
`vlog_recover`. 64 wrap-class (nunca ganham), 26 sem campo aeneas,
10 entry missing (7 L28 = cluster_real.rs, território P2.4/paralelo).
