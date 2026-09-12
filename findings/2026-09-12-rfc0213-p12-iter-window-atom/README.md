# RFC-0213 P1.2 5/6 — atom `catalog:iter_window` (`iter_window_keep_fate_iff`, Iter.lean)

Data: 2026-09-12. Par `iter_window` promovido de `data_fate` para
`atom`, teorema `iter_window_keep_fate_iff` em
`formal/aeneas/lean/Iter.lean` sobre o extrato Aeneas de
`rocksdb-compat/src/iter_kernel.rs` (`iter_window_keep`, entry do
catálogo).

## O que o teorema diz (fate forall sobre o corpo extraído)

`iter_window_keep snapshot_live = ok v` ↔ `v = snapshot_live` — o
corpo é a identidade: a janela de iteração da compat mantém a entrada
se e só se o snapshot ainda está vivo. Molde Bool-identity
(`cases <;> cases <;> simp`).

## Por que data_fate não é mais necessário

Corpo mínimo totalmente coberto pelo iff — nada fica ao sabor do par.
O as-is (`iter_window_keep_as_is` = `true` sempre) ressuscita
entradas escondidas depois que o snapshot morreu e é refutado ao
vivo: `tests::iter_window_keep_on_live_hidden_is_not_ok`
(1 passed, `cargo test --lib -p rocksdb-compat` — planta no
`lib.rs` da compat, não no kernel).

## Ratchet

- `close_proofs.tsv`: +1 `atom catalog:iter_window`
  (`iter_window_keep_fate_iff`, entry `iter_window_keep`).
- `proof_depth.tsv`: floor_atom 119→120, floor_extract 159→158,
  cap_data_fate 4→3.
- `catalog.json`: `data_fate` removido, `atom_reason` datado.
- `residuals.json`: atom+1, extract−1, glue.data_fate−1.
- Gate `check_depth_floor.py`: GREEN
  (extract=158, close=6, atom=120, count=7, data_fate=3).
