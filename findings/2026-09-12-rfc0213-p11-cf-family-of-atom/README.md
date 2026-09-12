# RFC-0213 P1.1 5/5 — atom `catalog:cf_family_of` (`cf_family_of_fate_iff`, Cf.lean) + FECHAMENTO P1.1

Data: 2026-09-12. Par `cf_family_of` promovido de `data_fate` para
`atom`, teorema `cf_family_of_fate_iff` em `formal/aeneas/lean/Cf.lean`
sobre o extrato Aeneas de `cf_kernel.rs`. Fecha a fatia P1.1
(flush ×3 + cf ×2).

## O que o teorema diz (fate forall sobre o corpo extraído)

`cf_family_of user_key = ok v` ↔ exatamente a rota extraída:

- `iter` ok + `position` do primeiro 0 ok;
- `none` ⇒ `v` é o `into` de `"default"` (`From<&str>` para `String`);
- `some i1` com `i1 > 0#usize` ⇒ `index ..i1` ok, `from_utf8_lossy`
  ok, `Cow.into_owned` ok decide `v` (bytes antes do primeiro NUL,
  decode lossy, owned);
- `some i1` sem overhang (NUL líder) ⇒ `into "default"` = `ok v`.

## Por que data_fate não é mais necessário

O corpo inteiro é coberto pelo iff ∃-cadeia; as folhas
(`from_utf8_lossy`, `Cow.into_owned`, `into`) são passos monádicos
declarados sobre o resultado — a fronteira de axiomas do extrato,
não sorte do par. O as-is (`cf_family_of_as_is` = `"default"` para
toda chave — familia nomeada perdida em bounds de SST) é refutado ao
vivo: `three_teeth_plants::cf_family_of_on_live_sst_bounds_is_not_ok`
(1 passed, `cargo test --lib -p pedradb-sim`).

## Ratchet

- `close_proofs.tsv`: +1 `atom catalog:cf_family_of`
  (`cf_family_of_fate_iff`, entry `cf_family_of`).
- `proof_depth.tsv`: floor_atom 115→116, floor_extract 163→162,
  cap_data_fate 9→8.
- `catalog.json`: `data_fate` removido, `atom_reason` datado.
- `residuals.json`: atom+1, extract−1, glue.data_fate−1.
- Gate `check_depth_floor.py`: GREEN
  (extract=162, close=6, atom=116, count=7, data_fate=8).

## FECHAMENTO P1.1 (totais da fatia)

cap 13→8, floor_atom 111→116, floor_extract 167→162, 5/5 atoms
(394a08d0 flush_publish, 399adf72 auto_flush_due, b07dcd27
flush_decision, 033b9bbb cf_family, este commit cf_family_of).

## Lições de prova desta fatia

- Onde o corpo termina em `if` (não em `match`), `whnf` desdobra
  demais (`ite` → `Decidable.rec` cru) e `split` não recupera. A
  saída robusta: `have h2 : (if COND then DO else ELSE) = ok v :=
  hval` (a escrita fecha por defeq através do redex) e então
  `by_cases` + `rw [if_pos/if_neg] at h2` sobre o `ite` escrito.
- `rw [if_pos hgt]` dispara sobre o `ite` do extrato com a condição
  escrita na grafia do statement (a instância Decidable do `>` de
  Usize é a mesma que o extrato usa).
