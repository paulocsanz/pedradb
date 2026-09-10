# RFC-0191 P2.3 (passo 9) — queued_leave_finish graduado a atom (cap 122→121)

**Data:** 2026-09-10 · **Fire:** 767 · **Estado:** landed

## Alvo

O par `queued_leave_finish` (RFC-0122/0123,
`pedradb-raft/src/membership_kernel.rs`, entry `queued_leave_finish_ok`)
estava parado em `data_fate`. A garantia: **um leave pendente termina
exatamente quando a entrada de leave não está no log ou já está
commitada** — um leave ainda no log e sem commit nunca termina (não
"termina por acaso"). Segunda graduação fora do store (Membership.lean).

## Teorema (atom registrado)

`queued_leave_finish_ok_iff_not_in_log_or_committed` (`Membership.lean`,
∀ `leave_in_log leave_committed`): `ok true` ↔
(`leave_in_log = false` ∨ `leave_committed = true`). 0 sorry.

### Lean lore (Fire 767)

- O RHS do `↔` é uma disjunção de **equações**, não de conjunções: o
  padrão do Fire 766 `⟨c1, h⟩` / `rintro (h1 | ⟨hl, hc⟩)` não se aplica
  — `⟨…,…⟩` sobre `Eq.refl` (0 campos explícitos) falha e o `hl`
  vira identificador desconhecido. Usar `exact Or.inr h` e
  `rintro (h1 | hc)`.
- No sentido ←, com `hc : leave_committed = true` a condição do `if`
  (`leave_in_log`) continua desconhecida: `split` e `rw [hc]` no ramo
  then, `rfl` no ramo else.
- `next c => tac1` de uma linha seguido de `tac2` na linha seguinte na
  MESMA coluna do `next` embaralha o escopo do bloco (erro "unsolved
  goals" + "No goals to be solved" aos pares): usar a forma multi-linha
  `next c =>` com corpo recuado.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 122 | **121** (`queued_leave_finish` gradua) |
| `floor_atom` | 9 | **10** |
| `floor_extract` / residuals `extract` | 270 | **269** (recount) |
| residuals `proof_depth` | 270/2/9/17 | **269/2/10/17** |
| residuals `data_fate`/`single_artifact` | 122/291 | **121/291** |

## Evidência

- `lake build Membership` → verde, 0 sorry (warnings só na Aeneas.Std).
- `cargo test -p pedradb-store --lib queued_leave_finish_ok_on_live_queued`
  → planta `three_teeth_queued::queued_leave_finish_ok_on_live_queued_is_not_ok`
  verde.
- depth-floor GREEN (extract 269, atom 10/10, data_fate 121≤121);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
