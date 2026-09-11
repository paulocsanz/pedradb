# RFC-0205 P0.1 — sexto close registrado: `may_publish_group` (publish-after-WAL ∀)

Data: 2026-09-11. Escada: floor_close 5→6, residuals
proof_depth.close 6→7 (6 registrados + 1 twin sem extração). Par
`group_publish` (`crates/pedradb-core/src/group_commit_kernel.rs`,
entry `may_publish_group`, chamado ao vivo por `finish_group_off_lock`
em `crates/pedradb-core/src/concurrent.rs`).

## O que foi pago

O coração publish-after-WAL do commit off-lock agora é um ∀
REGISTRADO no crédito da escada (GroupCommit.lean,
`may_publish_group_ok_iff_wal_io_ok`):

```lean
theorem may_publish_group_ok_iff_wal_io_ok :
    ∀ (wal_io_ok v : Bool),
      (may_publish_group wal_io_ok = ok v) ↔ (wal_io_ok = v)
```

O corpo extraído é o lift puro `ok wal_io_ok` — um grupo é publicado
exatamente quando o WAL I/O dele deu Ok; não existe terceiro destino.
O RFC sugeria o molde do close do bearer (fate-iff sobre cadeia de
callees), mas corpo SEM bind não tem cadeia: a forma honesta é o
molde pure-lift já registrado como atom
(`dir_sync_required_ok_iff_sync`, WriteAdmission.lean) — diferença de
molde documentada aqui, não silenciada.

Os dentes concretos existentes viram instâncias:
`may_publish_group_needs_wal_ok` (false→false) e o dente as-is
(`may_publish_group_as_is false = ok true` — o mutante publica com
WAL falho) continuam no mesmo arquivo como evidência do par.

Antes: a garantia "WAL falho não publica" vivia em dentes concretos
(ComposeConcurrent.lean, dois mundos pinados) + texto do residuals
(0071 P0). Agora: linha no registro
(`close catalog:group_publish …`), o mesmo lugar onde a escada lê o
crédito.

## Registro (mesmo commit)

- `close_proofs.tsv`: linha
  `close catalog:group_publish may_publish_group_ok_iff_wal_io_ok formal/aeneas/lean/GroupCommit.lean may_publish_group`
- `proof_depth.tsv`: floor_close 5→6
- `scripts/formal/residuals.json`: glue.proof_depth.close 6→7
- catálogo intocado (par de close não pede cirurgia; precedente
  occ_batch_plan) — `group_publish` não tem `data_fate`

## Verificação

- `lake build GroupCommit` verde (1699 jobs), zero `sorry`
- gates: depth-floor GREEN (registered close=6 floor 6, residuals
  close=7 == live 7/36, extract 242, data_fate 95≤95), product GREEN,
  ledger GREEN 299/266/33
- `scripts/lean_extracts.sh --required`: ok (61 libs + 12 compose)
- planta DST `may_publish_group_on_live_group_is_not_ok`
  (pedradb-core, concurrent.rs): 1 passed
