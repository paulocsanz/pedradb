# RFC-0202 P0.2 — fila lost-update: atom `occ_member_fate`

Data: 2026-09-11. Escada: cap_data_fate 97→96, floor_atom 34→35,
floor_extract 244→243 (par migra extract→atom).

## O que foi pago

Par `occ_member_fate` (`crates/pedradb-core/src/group_commit_kernel.rs`,
handlers `validate_occ_batch` — o caller link do ConcurrentDb). Corpo
extraído:

```lean
def occ_member_fate (too_old : Bool) (conflict : Bool) : Result OccMemberFate := do
  if too_old
  then ok OccMemberFate.TooOld
  else if conflict
       then ok OccMemberFate.Conflict
       else ok OccMemberFate.Ok
```

Teorema registrado (GroupCommit.lean):

```lean
theorem occ_member_fate_ok_iff_precedence :
    ∀ (too_old conflict : Bool) (f : OccMemberFate),
      (occ_member_fate too_old conflict = ok f) ↔
      ((too_old = true ∧ f = OccMemberFate.TooOld) ∨
       (too_old = false ∧ conflict = true ∧ f = OccMemberFate.Conflict) ∨
       (too_old = false ∧ conflict = false ∧ f = OccMemberFate.Ok))
```

Leitura: o destino do membro OCC segue EXATAMENTE a precedência TooOld
> Conflict > Ok — snapshot ilegível aborta TooOld mesmo se também houver
conflito; sem TooOld, conflito aborta Conflict; só sem ambos aplica Ok.
O AS-IS nunca-abortar (membro lagando comita — a lost-update lie) produz
`ok Ok` para todo input e é inalcançável. Prova: unfold + cases duplo +
simp [eq_comm] — 17 linhas, zero sorry, build verde primeira tentativa.

## Registro (mesmo commit)

- `close_proofs.tsv`: linha `atom catalog:occ_member_fate …`
- `catalog.json`: `data_fate` removido; `atom_reason` datado 2026-09-11
- `proof_depth.tsv`: floor_extract 243 / floor_atom 35 / cap_data_fate 96
- `residuals.json`: extract 243, atom 35, data_fate 96
- RFC-0202: P0.2 checkbox + row flip

## Verificação (capturas ao vivo)

- `lake build GroupCommit`: `Build completed successfully (1699 jobs)`
- gates: `depth-floor: GREEN — extract=243 (floor 243) / atom=35 (floor
  35), residuals 5/35 == live 5/35, count=7, data_fate=96<=96`;
  `product-floor: GREEN`; `ledger: GREEN — 299/266/33`
- `lean_extracts.sh --required`: `ok (61 libs + 12 compose)`
- planta DST: `occ_member_fate_on_live_conflict_is_not_ok` 1 passed /
  0 failed (kernel de produção)
