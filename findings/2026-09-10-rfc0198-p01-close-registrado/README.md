# RFC-0198 P0.1 — primeiro close de glue registrado

`wal_commit_plan ∘ fence_on_sync_fail` movido de extrato para degrau
FECHADO da escada. Peer de referência: nenhum (fatia formal, sem bench).

## O que mudou

- `formal/aeneas/lean/WriteAdmission.lean`: teorema
  `wal_commit_plan_ok_iff_fence_chain` — iff ∀ sobre os DOIS corpos
  extraídos (`WriteAdmissionKernel.lean`): o plano é `ok v` exatamente
  quando o fence aterrissa `ok b` e os dois ifs do plano roteiam
  `b`/`need_sync` para `AppendSyncFence` / `AppendSyncApplyOk` /
  `AppendApplyOk`. Helpers privados `bind_ok_inv` / `bind_intro`
  (√-mold do bind `Result` ok).
- `scripts/ratchet/close_proofs.tsv`: linha
  `close  catalog:wal_commit_plan  wal_commit_plan_ok_iff_fence_chain
  …  wal_commit_plan` (2º close registrado; 1º foi `merge_sift`).
- `scripts/ratchet/proof_depth.tsv`: `floor_extract` 248→247,
  `floor_close` 1→2 (mesmo commit da linha).
- `scripts/formal/residuals.json`: `glue.proof_depth` extract 248→247,
  close 2→3 (= live).
- Catálogo INALTERADO: close NÃO deleta `data_fate` (só graduation para
  atom deleta); cap 100 intacto.

## Verificação (mesmo commit)

- `lake build WriteAdmission` verde; `grep sorry` no arquivo = 0.
- `check_depth_floor.py` GREEN — extract=247 (floor 247), registered
  close=2 (floor 2), residuals 3 == live 3.
- `check_product_floor.py` GREEN — D1=close R1=atom T1=atom C1=close.
- `check_ledger_consistency.py` GREEN — 12 ponteiros, 298/265/33.

## Erro encontrado no caminho

Primeiro build falhou em `rw [if_pos hb, hv]` no caso `b = true`: `hb` é
o par `(b = true ∧ v = …)`, não a hipótese do if — correto é
`rw [if_pos hb.1, hb.2]`.
