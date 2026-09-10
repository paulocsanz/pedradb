# RFC-0191 P2.3 (passo 14) — sched_plant_joint graduado a atom (cap 117→116)

**Data:** 2026-09-10 · **Fire:** 772 · **Estado:** landed

## Alvo

O par `sched_plant_joint` (RFC-0068, `pedradb-store/src/
membership_kernel.rs` — o kernel STORE, não o raft; entry
`plant_joint_schedule_ok`, caller `pedradb-world`) estava parado em
`data_fate`. A garantia: **uma planta de joint-commit é ok exatamente
quando o mundo opt-in emite E o default ainda omite** — a planta é
visível só para o leitor opted-in.

## Teorema (atom registrado)

`plant_joint_schedule_ok_iff_opt_in_emits_and_default_omits`
(`StoreMembership.lean`, ∀ `opt_in_emits default_omits`): `ok true` ↔
(`opt_in_emits = true ∧ default_omits = true`). 0 sorry, verde na
primeira build. Extrato `if opt_in_emits then ok default_omits else ok
false` — shape 767 com RHS conjuntivo: forward `⟨c1, h⟩`, else-ramo
`absurd h (by simp)` (`ok false = ok true`), backward
`rw [if_pos h1, h2]`.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 117 | **116** (`sched_plant_joint` gradua) |
| `floor_atom` | 14 | **15** |
| `floor_extract` / residuals `extract` | 265 | **264** (recount) |
| residuals `proof_depth` | 265/2/14/17 | **264/2/15/17** |
| residuals `data_fate`/`single_artifact` | 117/291 | **116/291** |

## Evidência

- `lake build StoreMembership` → verde, 0 sorry.
- `cargo test -p pedradb-world --lib
  world_opt_in_schedule_emits_plant_committed_joint` → planta verde.
- depth-floor GREEN (extract 264, atom 15/15, data_fate 116≤116);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
