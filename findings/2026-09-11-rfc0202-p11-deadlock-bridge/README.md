# RFC-0202 P1.1 — fila deadlock: ponte `wait_for_deadlock` (arestas de passo)

Data: 2026-09-11. Escada: cap_data_fate 96→95, floor_atom 35→36,
floor_extract 243→242. Par `wait_for_deadlock`
(`crates/rocksdb-compat/src/locktab.rs`, 2PL locktab) promovido a atom.

## O que foi pago

O corpo do loop extraído (LocktabKernel.lean) chama `HashMap.get` /
`HashSet.insert` como axiomas — o iff completo "detector dispara ↔
existe ciclo" não é provável do extract (fronteira datada em
`formal/aeneas/EXTRACT.md`). O núcleo provável, pago agora: as TRÊS
arestas de saída de um passo do detector, com hipóteses de lookup
fixando os resultados das chamadas-axioma (Locktab.lean):

```lean
theorem wait_for_deadlock_step_nowait_is_alive :
    ∀ (owned waiting waiter owner seen seen'), …insert … = ok (true, seen') →
      …get … waiting owner = ok none →
      wait_for_deadlock_loop.body … = ok (ControlFlow.done false)

theorem wait_for_deadlock_step_cycle_closes :
    ∀ (… k), …insert … = ok (true, seen') →
      …get … waiting owner = ok (some k) →
      …get … owned k = ok (some waiter) →
      wait_for_deadlock_loop.body … = ok (ControlFlow.done true)

theorem wait_for_deadlock_step_revisit_reports_cycle :
    ∀ (…), …insert … = ok (false, seen') →
      wait_for_deadlock_loop.body … = ok (ControlFlow.done true)
```

Leitura: quem não espera por ninguém é reportado VIVO (o detector nunca
inventa ciclo sem aresta de espera); cadeia que fecha no próprio waiter
é reportada CICLO; revisit é reportado CICLO (o detector termina, nunca
loops). O dente as-is (detector desligado, `ok false`) já existia
(`wait_for_deadlock_as_is_dente`).

Lição de prova (registrada no EXTRACT.md): `rw` sozinho não reduz os
binds do do-block (transparência do rfl automático do rewrite); `simp`
fecha as arestas sem lookup encadeado; `simp [hkey]` normaliza o
do-block e aí reescreve o lookup exposto. Enunciados na forma `∀ … →`
(binders diretos não passam o gate do registro, que exige ∀ no texto).

## Registro (mesmo commit)

- `close_proofs.tsv`: linha `atom catalog:wait_for_deadlock
  wait_for_deadlock_step_cycle_closes … Locktab.lean`
- `catalog.json`: `data_fate` removido; `atom_reason` datado 2026-09-11
- `proof_depth.tsv`: floor_extract 242 / floor_atom 36 / cap_data_fate 95
- `residuals.json`: extract 242, atom 36, data_fate 95
- `formal/aeneas/EXTRACT.md`: fronteira datada (semântica de mapa = TCB)
- RFC-0202: P1.1 checkbox + row flip

## Verificação (capturas ao vivo)

- `lake build Locktab`: `Build completed successfully (1699 jobs)`;
  `grep -c sorry Locktab.lean` = 0
- gates: `depth-floor: GREEN — extract=242 (floor 242) / atom=36 (floor
  36), residuals 5/36 == live 5/36, count=7, data_fate=95<=95`;
  `product-floor: GREEN`; `ledger: GREEN — 299/266/33`
- `lean_extracts.sh --required`: `ok (61 libs + 12 compose)`
- planta DST: `cargo test -p rocksdb-compat --lib wait_for_deadlock` →
  `wait_for_deadlock_on_live_cycle_is_not_ok` 1 passed / 0 failed
