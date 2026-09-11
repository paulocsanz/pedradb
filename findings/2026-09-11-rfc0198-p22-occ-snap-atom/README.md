# RFC-0198 P2.2 — primeira descida de cap do RFC: occ_snap_uses_published a atom

Primeiro atom data-fate do RFC-0198 (cap_data_fate 100→99, mesma
receita um-por-commit do P2.3/0191). O if promovido decide a BASE DE
VISIBILIDADE do snapshot OCC na janela off-lock: seq publicada quando
há commit inflight, last_seq caso contrário.

## O que o teorema diz

`occ_snap_uses_published_ok_iff_inflight` (Flush.lean, zero sorry):

```
∀ (commit_inflight v : Bool),
  (occ_snap_uses_published commit_inflight = ok v) ↔ (commit_inflight = v)
```

Corpo extraído (FlushKernel.lean): `do ok commit_inflight` — lift puro;
a iff é a regra de computação inteira do do-block (precedente
`dir_sync_required_ok_iff_sync`, RFC-0191 P2.3 passo 30, molde copiado).

## Registro (mesmo commit)

- `close_proofs.tsv`: `atom catalog:occ_snap_published
  occ_snap_uses_published_ok_iff_inflight formal/aeneas/lean/Flush.lean
  occ_snap_uses_published`
- `catalog.json`: `"data_fate": true` REMOVIDO do par
  `occ_snap_published` (par sai do pool data-fate TCB; `three_teeth:
  true` mantém AS-IS `occ_snap_uses_published_as_is` + twin + dst_plant
  exigidos e presentes; caller `concurrent.rs`/handler `occ_snapshot`
  seguem válidos — check estático de conteúdo commitado)
- `proof_depth.tsv`: `floor_extract 247→246`, `floor_atom 31→32`,
  `cap_data_fate 100→99`
- `residuals.json` (replaces cirúrgicos): `extract 247→246`,
  `atom 31→32`, `data_fate 100→99`

## Verificação (mesmo commit)

- `lake build Flush` verde; `grep sorry` = 0; `lean_extracts
  --required` verde (61 libs + 6 compose).
- Gates GREEN: depth `extract=246 (floor 246), close=3 (floor 3),
  atom=32 (floor 32), residuals 4/32 == live, data_fate=99<=99`;
  product; ledger `298/265/33`.
- `pedra_formal.py`: `2647 ok, 0 gap` (os 161 fail são o set herdado
  de lint/spawn em arquivos não tocados — nenhum novo).
- Planta DST nomeada passa:
  `cargo test -p pedradb-core --lib --
  flush_kernel::tests::occ_snap_uses_published_on_live_inflight_is_not_ok`
  → 1 passed.

## Preparação preservada

O teorema e o runbook completo foram preparados em
`findings/2026-09-11-rfc0198-p22-occ-snap-atom-prep/README.md` durante
o bloqueio dos compartilhados (mesma condição do P0.2); aterrissados
aqui verbatim após o commit `4df272d5` da sessão paralela.
