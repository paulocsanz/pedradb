# RFC-0213 P0.1 5/9 — átomo `catalog:wal_commit_plan`

Data: 2026-09-12. Quinta promoção de P0.1 (cadência write_admission
×9). Escada viva: floor_atom 102→103, floor_extract 176→175,
cap_data_fate 22→21 (data_fate 22→21); residual close 7→6 (ver
abaixo).

## O que foi pago

Teorema `wal_commit_plan_fate_iff` em
`formal/aeneas/lean/WriteAdmission.lean`: fate ∀ sobre o corpo
extraído — o resultado é `AppendSyncFence` exatamente em
`need_sync ∧ sync_failed`, `AppendSyncApplyOk` exatamente em
`need_sync ∧ ¬sync_failed`, `AppendApplyOk` exatamente em `¬need_sync`.

## Semântica

O plano de append do WAL tem três ramos sem sobreposição: cerca
(fence) quando o sync exigido falhou; sync+apply-ok quando o sync
exigido deu certo; apply puro quando nenhum sync era exigido.

## AS-IS recusado

O twin as-is devolve o plano errado sob sync exigido+falhado (ver
planta) — responderia como se durável.

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`wal_commit_plan_on_live_sync_fail_is_not_ok` — verde (1 passed;
942 filtered).

## Cirurgia de catálogo

`promote_atom.py wal_commit_plan`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals re-carimbados.

## Escada — degrau close→atom

O par JÁ tinha linha `close` registrada (e linha `count`). Com a
linha `atom` nova, o `registered_map` do ratchet (última escrita
vence) passa a contar o par no degrau atom — o crédito de escada
migra close→atom no MESMO commit: linha close mantida (o teorema
close continua verdadeiro e a contagem de linhas fecha com
floor_close=6), residuals `proof_depth.close` 7→6. O gate
`check_depth_floor.py` provou o vermelho antes do ajuste ("stale
residual") e o verde depois.

floor_atom 102→103, floor_extract 176→175, cap_data_fate 22→21.
Gate GREEN: extract=175 (floor 175), registered close=6 (floor 6) /
atom=103 (floor 103), residuals close=6/atom=103 == live 6/103,
data_fate=21<=21.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).

## Nota de prova

O corpo extraído faz `let b ← fence_on_sync_fail …` antes dos ifs;
o `unfold wal_commit_plan fence_on_sync_fail` + `cases` ×2 + `simp`
resolve, sobrando apenas `X = r ↔ r = X`, fechado por `eq_comm`.
