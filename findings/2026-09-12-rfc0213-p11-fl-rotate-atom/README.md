# RFC-0213 P1.1 3/5 — átomo `catalog:flush_decision`

Data: 2026-09-12. Terceira promoção de P1.1 (cadência flush ×3 +
cf ×2). Escada viva: floor_atom 113→114, floor_extract 165→164,
cap_data_fate 11→10 (data_fate 11→10).

## O que foi pago

Teorema `flush_decision_fate_iff` em
`formal/aeneas/lean/Flush.lean`: fate ∀ sobre o corpo extraído —
`RotateWal` exatamente com os cinco travadores soltos
(mem_empty ∧ ¬imm_present ∧ ¬pin_live ∧ ¬parked_unflushed ∧
¬commit_inflight), `KeepWal` exatamente caso contrário.

## Semântica

O WAL só rota quando o pipeline está totalmente quiescente: qualquer
retenção (imm presente, pin vivo, dados estacionados sem flush,
commit em voo, memtable povoada) mantém o segmento ativo.

## AS-IS recusado

O twin as-is ignora o pin vivo (`wal_rotate_decision_as_is_ignore_pin`)
— rotaria o WAL sob um pin ativo, cortando leitura em andamento.

## Planta DST

`crates/pedradb-core/src/flush_kernel.rs` (mod tests):
`wal_rotate_decision_on_live_pin_is_not_ok` — verde (1 passed;
942 filtered).

## Cirurgia de catálogo

`promote_atom.py flush_decision` (entry `wal_rotate_decision`):
linha `atom` no `close_proofs.tsv`; `atom_reason` datado no
catálogo; `data_fate` removido; residuals/proof_depth
re-carimbados. Nota: a linha `close` de `catalog:wal_rotate_decision`
pertence a outro par do catálogo (id distinto) — nenhuma migração
de degrau aqui; residual close permanece 6.

## Escada

floor_atom 113→114, floor_extract 165→164, cap_data_fate 11→10.
Gate `check_depth_floor.py` GREEN: extract=164 (floor 164),
atom=114 (floor 114), data_fate=10<=10, residuals == live.

## Gates

`lake build Flush` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …Flush.lean | grep -c "^+theorem"` = 1).

## Nota de prova

`rcases` abre o registro `WalPinState` nos cinco campos Bool;
`cases` ×5 (32 ramos) + `simp` + `eq_comm` fecha.
