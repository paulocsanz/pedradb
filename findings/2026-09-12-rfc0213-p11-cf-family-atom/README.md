# RFC-0213 P1.1 4/5 — atom `catalog:cf_family` (`cf_family_fate_iff`, Cf.lean)

Data: 2026-09-12. Par `cf_family` promovido de `data_fate` para
`atom` no catálogo (`scripts/formal/catalog.json`), teorema
`cf_family_fate_iff` em `formal/aeneas/lean/Cf.lean` sobre o extrato
Aeneas de `cf_kernel.rs` (`key_in_cf_family`).

## O que o teorema diz (fate forall sobre o corpo extraído)

`key_in_cf_family user_key family = ok v` ↔ exatamente a rota extraída:

- **default** (`Str.eq family "default" = ok b`, `b = true`): `position`
  acha o primeiro 0; `none` ⇒ `v = true`; índice 0 ⇒ `v = true`;
  senão o prefixo até o índice deve ser byte-a-byte `default`
  (`CmpPartialEqArray.eq` sobre `Array.make 7 [100,101,102,97,117,108,116]`)
  decidindo `v`.
- **nomeada** (`¬(b = true)`): `family` overhanga estritamente
  (`len user_key > len bytes`), `starts_with` decide o ramo, e o byte
  logo após o overhang é o 0 (`v = decide (i3 = 0#u8)`); sem overhang
  ⇒ `v = false`.

## Por que data_fate não é mais necessário

O corpo inteiro é coberto pelo iff ∃-cadeia (binds `eq`/`iter`/
`position`/`index`/`starts_with`/`index_usize` explícitos); a única
"sorte" restante é a igualdade de array no fim do ramo default —
axiomatizada no extrato, não sob controle do par. Planta DST refuta o
as-is (`key_in_cf_family_as_is` = `ok true` para toda chave):
`three_teeth_plants::key_in_cf_family_on_live_scan_is_not_ok`
(1 passed, `cargo test --lib -p pedradb-sim`).

## Ratchet

- `scripts/ratchet/close_proofs.tsv`: +1 linha `atom catalog:cf_family`
  (teorema `cf_family_fate_iff`, entry `key_in_cf_family`).
- `scripts/ratchet/proof_depth.tsv`: floor_atom 114→115,
  floor_extract 164→163, cap_data_fate 10→9.
- `scripts/formal/catalog.json`: `data_fate` removido,
  `atom_reason` datado.
- `scripts/formal/residuals.json`: atom+1, extract−1, glue.data_fate−1.
- Gate `scripts/check_depth_floor.py`: GREEN
  (extract=163, close=6, atom=115, count=7, data_fate=9).

## Lições de prova desta fatia

- `let (o, _) := (some i1, u)` (beta-redex do bind de position) não
  cede a `simp only []`, `dsimp only` nem `split`; `conv at h =>
  lhs; whnf` reduz beta→iota→iota até `match ↑i1` e aí `split at h`
  funciona.
- Backward com disjunções aninhadas: deixar o `rintro` plano
  (`hlast`) e fazer `rcases` DENTRO de cada bullet — o padrão
  profundo no `rintro` gera N>2 goals e desalinha bullets.
- Statement `v = (i3 = 0#u8)` elabora como Prop-eq; o corpo extraído
  é `ok (decide (i3 = 0#u8))` ⇒ escrever `v = decide (i3 = 0#u8)`.
