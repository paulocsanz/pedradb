# RFC-0202 P0.1 — fila data-race: atom `rwlock_client_may_mutate`

Data: 2026-09-11. Escada: cap_data_fate 98→97, floor_atom 33→34,
floor_extract 245→244 (par migra extract→atom).

## O que foi pago

Par `rwlock_client_may_mutate` (`crates/pedradb-core/src/group_commit_kernel.rs`,
handler `finish_group_off_lock`) — o protocolo do CLIENTE write-lock da
fila data-race do mapa de concorrência. Corpo extraído:

```lean
def rwlock_client_may_mutate (holding_write : Bool) : Result Bool := do
  ok holding_write
```

Teorema registrado (GroupCommit.lean):

```lean
theorem rwlock_client_may_mutate_ok_iff_holding_write :
    ∀ (holding_write v : Bool),
      (rwlock_client_may_mutate holding_write = ok v) ↔ (v = holding_write)
```

Leitura: mutação de `Db` permitida EXATAMENTE enquanto o cliente segura
a write-guard (CapybaraKV RW-lock client). O AS-IS (mutar depois de
soltar a guarda — a "data-race lie") produz `ok true` para qualquer
input e por isso é inalcançável pelo iff do kernel real. Prova: unfold +
simp [eq_comm] — 12 linhas, zero sorry.

## Registro (mesmo commit)

- `close_proofs.tsv`: linha `atom catalog:rwlock_client_may_mutate …`
- `catalog.json`: `data_fate` removido; `atom_reason` datado 2026-09-11
- `proof_depth.tsv`: floor_extract 244 / floor_atom 34 / cap_data_fate 97
- `residuals.json`: extract 244, atom 34, data_fate 97
- RFC-0202: P0.1 checkbox + row flip

## Verificação (capturas ao vivo)

- `lake build GroupCommit`: `Build completed successfully (1699 jobs)`;
  `grep -c sorry GroupCommit.lean` = 0 (build verde primeira tentativa)
- gates: `depth-floor: GREEN — extract=244 (floor 244) / atom=34 (floor
  34), residuals 5/34 == live 5/34, count=7, data_fate=97<=97`;
  `product-floor: GREEN`; `ledger: GREEN — 299/266/33`
- `lean_extracts.sh --required`: `ok (61 libs + 12 compose)`
- planta DST: `cargo test -p pedradb-core --lib rwlock_client_may_mutate`
  → `rwlock_client_may_mutate_on_live_off_lock_is_not_ok` 1 passed / 0
  failed (kernel de produção)
