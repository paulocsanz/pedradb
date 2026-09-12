# RFC-0213 P0.1 9/9 — átomo `catalog:pit_resync_rewrite` (FECHAMENTO P0.1)

Data: 2026-09-12. Nona e última promoção de P0.1 (cadência
write_admission ×9). Escada viva: floor_atom 106→107,
floor_extract 172→171, cap_data_fate 18→17 (data_fate 18→17).

## O que foi pago

Teorema `pit_resync_rewrite_fate_iff` em
`formal/aeneas/lean/WriteAdmission.lean`: fate ∀ sobre o corpo
extraído — `(pit_resync_needs_rewrite is_resync = ok v) ↔
v = is_resync`.

## Semântica

Um resync point-in-time precisa do rewrite EXATAMENTE quando o
registro é um resync — o veredito é a identidade sobre a flag.

## AS-IS recusado

O twin as-is devolve `false` — pula o rewrite de resync, deixando o
estado point-in-time desincronizado.

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`pit_resync_needs_rewrite_on_live_resync_is_not_ok` — verde
(1 passed; 942 filtered).

## Cirurgia de catálogo

`promote_atom.py pit_resync_rewrite` (entry
`pit_resync_needs_rewrite`): linha `atom` no `close_proofs.tsv`;
`atom_reason` datado no catálogo; `data_fate` removido;
residuals/proof_depth re-carimbados.

## Escada

floor_atom 106→107, floor_extract 172→171, cap_data_fate 18→17.
Gate `check_depth_floor.py` GREEN: extract=171 (floor 171),
atom=107 (floor 107), data_fate=17<=17, residuals == live.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).

## Totais do P0.1 (fechamento)

- 9/9 átomos: write_admission, write_admit, seq_exhausted,
  fence_on_sync_fail, wal_commit_plan, torn_head_empty_log,
  torn_tail_needs_cut, seq_after_feed, pit_resync_rewrite.
- cap_data_fate 26→17, floor_atom 98→107, floor_extract 180→171.
- Residual close 7→6 (wal_commit_plan já tinha linha close
  registrada; subiu o degrau close→atom no mesmo commit).
- 9 commits, 1 teorema `_fate_iff` por commit; todas as plantas
  DST do write_admission_kernel verdes no fechamento.
