# RFC-0200 P2.1 — primeiro atom data-fate do ciclo: `wal_sync_required`

Data: 2026-09-11. Escada: cap_data_fate 99→98, floor_atom 32→33,
floor_extract 246→245 (o par migra do degrau extract para atom).

## O que foi pago

Par `wal_sync_required` (`crates/pedradb-core/src/write_admission_kernel.rs`,
handler `commit_ops_with`, caller `db.rs`) — a decisão de `fdatasync` do WAL
antes do Ok em CADA commit do write path. Corpo extraído (Aeneas,
WriteAdmissionKernel.lean):

```lean
def wal_sync_required
  (client_set : Bool) (client_sync : Bool) (db_sync : Bool) : Result Bool := do
  if client_set
  then ok client_sync
  else ok db_sync
```

Teorema registrado (WriteAdmission.lean):

```lean
theorem wal_sync_required_ok_iff_client_else_db :
    ∀ (client_set client_sync db_sync v : Bool),
      (wal_sync_required client_set client_sync db_sync = ok v) ↔
      (if client_set then v = client_sync else v = db_sync)
```

Leitura: a resolução do knob de sync é total e single-valued — o commit
requer barrier exatamente na escolha EXPLÍCITA do cliente quando ele setou
`WriteOptions.sync`, senão no default do db. O AS-IS (`sempre false` — ack
sem barrier) fica do outro lado do iff: nenhum input o satisfaz. Prova:
unfold + cases no client_set + simp [eq_comm] — 16 linhas, zero sorry.

## Registro (mesmo commit)

- `close_proofs.tsv`: linha `atom catalog:wal_sync_required …wal_sync_required`
- `catalog.json`: `data_fate` removido do par; `atom_reason` datado 2026-09-11
- `proof_depth.tsv`: floor_extract 245 / floor_atom 33 / cap_data_fate 98
- `residuals.json`: extract 245, atom 33, data_fate 98
- RFC-0200: P2.1 checkbox + row flip

## Verificação (capturas ao vivo)

- `lake build WriteAdmission`: `Build completed successfully (1699 jobs)`;
  `grep -c sorry WriteAdmission.lean` = 0
- gates: `depth-floor: GREEN — extract=245 (floor 245), registered ladder
  close=4 (floor 4) / atom=33 (floor 33), residuals close=5/atom=33 == live
  5/33, count=6, data_fate=98<=98`; `product-floor: GREEN — promoted=4>=floor
  4`; `ledger: GREEN — total=299 proof=266 campaign=33`
- `scripts/lean_extracts.sh --required`: `ok lean extracts (61 libs + 11 compose)`
- planta DST (três dentes, kernel real): `cargo test -p pedradb-core --lib
  wal_sync_required` → `wal_sync_required_on_live_client_true_is_not_ok ... ok`
  (1 passed; 0 failed)

Ledger `docs/verification-ledger.md` não muda nesta promoção (par não muda
total/kind/single_artifact; mesma forma do ade73704).
