# RFC-0198 P2.2 — prep do primeiro atom df (occ_snap_uses_published)

Prep NÃO-commitado (teorema já está em `formal/aeneas/lean/Flush.lean`,
build verde, sorry 0). O landing espera os arquivos de registro
compartilhados (`proof_depth.tsv`/`residuals.json`) assentarem — mesma
condição do P0.2 (monitor `01a08e67`).

## Teorema (já no tree, molde passo-30/dir_sync_required)

```lean
/-- RFC-0198 P2.2 (first cap-descent if): the OCC snapshot reads the
    published seq exactly when a commit is inflight — the computation
    rule of the do-block body (a pure lift; the caller's off-lock window
    decides the snapshot's visibility base). -/
theorem occ_snap_uses_published_ok_iff_inflight :
    ∀ (commit_inflight v : Bool),
      (occ_snap_uses_published commit_inflight = ok v) ↔ (commit_inflight = v) := by
  intro commit_inflight v
  unfold occ_snap_uses_published
  constructor
  · intro h
    injection h with _
  · intro h
    rw [h]
```

Corpo extraído (`FlushKernel.lean`): `def occ_snap_uses_published
(commit_inflight : Bool) : Result Bool := do ok commit_inflight` — lift
puro; a iff é a regra de computação inteira do do-block (precedente
`dir_sync_required_ok_iff_sync`, RFC-0191 P2.3 passo 30).

## Runbook do landing (UM commit, tudo junto)

1. Re-verificar floors atuais no TSV (a sessão paralela pode ter movido
   os números — recalibrar as setas abaixo).
2. `scripts/ratchet/close_proofs.tsv`: linha
   `atom\tcatalog:occ_snap_published\tocc_snap_uses_published_ok_iff_inflight\tformal/aeneas/lean/Flush.lean\tocc_snap_uses_published`
   (separador TAB, colunas kind/catalog_id/theorem/lean_file/entry).
3. `scripts/formal/catalog.json`: remover `"data_fate": true` do par
   `occ_snap_published` (o par mantém `three_teeth: true` — AS-IS
   `occ_snap_uses_published_as_is`, twin, dst_plant seguem exigidos por
   three_teeth; caller `concurrent.rs`/handler `occ_snapshot` seguem
   válidos, check estático de conteúdo commitado).
4. `scripts/ratchet/proof_depth.tsv`: `floor_extract 247→246`,
   `floor_atom 31→32`, `cap_data_fate 100→99` (mesma receita passo 30:
   par sai do pool extract, cap desce com o atom NO MESMO commit).
5. `scripts/formal/residuals.json` (replace cirúrgico, NUNCA
   sort_keys=True): `glue.proof_depth.extract 247→246`, `atom 31→32`,
   `glue.data_fate 100→99`.
6. Gates: depth GREEN (`extract=246 (floor 246), close=2, atom=32
   (floor 32), residuals == live, data_fate=99<=99`), product GREEN,
   ledger GREEN; `bash scripts/lean_extracts.sh --required` verde.
7. `cargo test` do plant nomeado:
   `occ_snap_uses_published_on_live_inflight_is_not_ok` (flush_kernel.rs).
8. Flip RFC-0198 P2.2 (checkbox + row) + findings de landing + commit
   por pathspec (Flush.lean, close_proofs.tsv, catalog.json,
   proof_depth.tsv, residuals.json, RFC, findings) + Fire no journal.

## Por que este par

- Corpo lift puro (prova trivial, zero risco de sorry).
- `concurrent.rs` no caller é check estático de conteúdo já commitado —
  o pair não depende do voo não-commitado dela.
- Tem AS-IS dente (`occ_snap_uses_published_as_is` → `ok false`) e
  planta DST nomeada — os dentes do par continuam cobertos por
  `three_teeth` após a graduação.
- O if é data-fate de verdade: decide a BASE DE VISIBILIDADE do
  snapshot OCC (seq publicada vs last_seq) na janela off-lock.
