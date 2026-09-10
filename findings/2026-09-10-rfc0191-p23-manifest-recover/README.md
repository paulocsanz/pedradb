# RFC-0191 P2.3 (passo 17) — manifest_recover graduado a atom (cap 114→113)

**Data:** 2026-09-10 · **Fire:** 775 · **Estado:** landed

## Alvo

O par `manifest_recover` (F196/G8, `pedradb-core/src/
manifest_kernel.rs`, entry `sst_recover_action`, handler
`recover_ssts` em db.rs) estava parado em `data_fate`. A garantia:
**recovery recusa abrir exatamente quando o manifest está Corrupt, OU
é Inventory com SST listado faltando no disco** — manifest ausente
rescanneia e instala; inventário completo serve; dano ou arquivo
faltando nunca serve.

## Teorema (atom registrado)

`sst_recover_action_refuse_iff_corrupt_or_inventory_missing`
(`Manifest.lean`, ∀ `obs listed`): `ok RefuseOpen` ↔ (`obs = Corrupt`
∨ (`obs = Inventory ∧ ∃ i, listed = Missing i)). 0 sorry, verde na
primeira build. Padrão 774 (cases duplo + rintro): forward `cases obs`
(Absent → absurd; Corrupt → `Or.inl rfl`; Inventory → `cases listed`,
AllPresent → absurd, `Missing i` → `Or.inr ⟨rfl, i, rfl⟩`); backward
`rw [hc]` / `rw [hinv, hi]` — os matches iota-reduzem por defeq no rfl
do próprio `rw`.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 114 | **113** (`manifest_recover` gradua) |
| `floor_atom` | 17 | **18** |
| `floor_extract` / residuals `extract` | 262 | **261** (recount) |
| residuals `proof_depth` | 262/2/17/17 | **261/2/18/17** |
| residuals `data_fate`/`single_artifact` | 114/291 | **113/291** |

## Evidência

- `lake build Manifest` → verde, 0 sorry.
- `cargo test -p pedradb-core --lib
  sst_recover_action_on_live_missing_sst` → planta (no próprio kernel)
  verde.
- depth-floor GREEN (extract 261, atom 18/18, data_fate 113≤113);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
