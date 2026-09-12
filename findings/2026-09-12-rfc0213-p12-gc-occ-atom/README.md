# RFC-0213 P1.2 6/6 — atom `catalog:occ_batch_plan` (`occ_batch_plan_fate_iff`, GroupCommit.lean)

Data: 2026-09-12. Par `occ_batch_plan` promovido de `close` para
`atom` (escada close→atom, precedente `wal_commit_plan` P0.1),
teorema `occ_batch_plan_fate_iff` em
`formal/aeneas/lean/GroupCommit.lean` sobre o extrato Aeneas de
`crates/pedradb-core/src/group_commit_kernel.rs` (`occ_batch_plan`,
entry do catálogo).

## O que o teorema diz (fate forall sobre o corpo extraído)

`occ_batch_plan too_old reads last_seq = ok v` ↔ ∃ n tal que
(n é o menor dos dois inputs, pela disjunção exata do `if` extraído)
∧ `occ_batch_plan_loop too_old reads last_seq n (with_capacity n)
0#usize = ok v` — o plano do grupo decide EXATAMENTE pela rota
min-len: o tamanho é o menor input e todo fate do membro vem do loop
extraído semeado com o vetor vazio de capacidade n. O `if` de `n` é
puro-lift (duas asas `ok`), provado com `bind_ok_inv`/`bind_intro`
já privados no wrapper; o loop `@[rust_loop]` fica como equação
existencial medida — o mesmo molde dos átomos de leveling
(`pick_l0_to_l1_fate_iff`).

## Por que data_fate não é mais necessário

O close RFC-0198 (`occ_batch_plan_member_fate_iff`) caraterizava só o
glue POR MEMBRO (o bind `occ_conflict >>= occ_member_fate`); a decisão
do extrato inteiro ficava ao sabor do par. Este iff cobre o corpo
extraído completo — resta só a equação do loop, que é hipótese medida,
não folclore. O as-is (`occ_batch_plan_as_is`) comete o membro
lagging e é refutado ao vivo:
`occ_batch_plan_on_live_lagging_is_not_ok`
(1 passed, `cargo test --lib --manifest-path
crates/pedradb-core/Cargo.toml`).

## Ratchet

- `close_proofs.tsv`: +1 `atom catalog:occ_batch_plan`
  (`occ_batch_plan_fate_iff`, entry `occ_batch_plan`). A linha
  `close` do RFC-0198 é MANTIDA (a contagem de linhas do gate segue
  ≥ floor_close; o crédito da escada migra via residual).
- `proof_depth.tsv`: floor_atom 120→121, floor_extract 158→157,
  cap_data_fate 3→2 (floor_close 6 inalterado).
- `catalog.json`: `data_fate` removido, `atom_reason` datado.
- `residuals.json`: atom+1, extract−1, close 6→5 (par com linha close
  registrada — o mesmo commit carrega a migração), glue.data_fate−1.
- Gate `check_depth_floor.py`: GREEN
  (extract=157, ladder close=6 linhas, atom=121, residuals
  close=5/atom=121 == live 5/121, count=7, data_fate=2).

## Lições

- Fechamento P1.2: cap 8→2, floor_atom 116→121, floor_extract
  162→157 (metas corrigidas pela nota datada do 3/6 — visible_at
  cap-only).
- `Slice` ambíguo no GroupCommit.lean (`open Aeneas Std`): o extrato
  abre `Aeneas Aeneas.Std` — qualificar `Aeneas.Std.Slice` no
  enunciado resolve.
- A promessa do close de 1998-era era por membro; o atom do 0213 é o
  extrato inteiro — a escada close→atom custa exatamente 1 residual
  de close no mesmo commit, sem mexer no floor.
