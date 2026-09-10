# RFC-0191 P2.3 (passo 15) — blob_gc_pick graduado a atom (cap 116→115)

**Data:** 2026-09-10 · **Fire:** 773 · **Estado:** landed

## Alvo

O par `blob_gc_pick` (F-active-gen, `pedradb-core/src/
vlog_gc_kernel.rs`, entry `blob_gc_action`, handler
`compact_blob_auto`) estava parado em `data_fate`. A garantia: **o GC
de blob reescreve exatamente quando o gen está inativo E ainda tem
bytes** — gen ativo nunca é reescrito; inativo vazio não tem o que
reescrever.

## Teorema (atom registrado)

`blob_gc_action_rewrite_iff_inactive_with_bytes` (`VlogGc.lean`, ∀
`is_active bytes`): `ok BlobGcAction.Rewrite` ↔ (`is_active = false ∧
bytes > 0#u64`). 0 sorry, verde na primeira build. Dois ifs
encadeados: forward `split at h` duas vezes (ramos Skip fecham com
`absurd h (by simp)` — construtor ≠ construtor); o `next c2` do if de
U64 traz o **Prop** `bytes > 0#u64` direto (útil na conjunção);
backward `rw [if_neg (by simp [h1]), if_pos h2]` numa linha só.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 116 | **115** (`blob_gc_pick` gradua) |
| `floor_atom` | 15 | **16** |
| `floor_extract` / residuals `extract` | 264 | **263** (recount) |
| residuals `proof_depth` | 264/2/15/17 | **263/2/16/17** |
| residuals `data_fate`/`single_artifact` | 116/291 | **115/291** |

## Evidência

- `lake build VlogGc` → verde, 0 sorry.
- `cargo test -p pedradb-core --lib blob_gc_action_on_live_active`
  → planta (no próprio kernel) verde.
- depth-floor GREEN (extract 263, atom 16/16, data_fate 115≤115);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
