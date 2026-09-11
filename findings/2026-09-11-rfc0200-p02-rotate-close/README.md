# RFC-0200 P0.2 — quarto close de glue registrado (`wal_rotate_decision ∘ wal_segment_is_empty`)

Commit: (este slice) `Flush.lean` + catalog pair + registro + RFC-0200.

## O par

Board compose (`unpaid_compose=0/17`): `try_rotate_wal` com
plan=`wal_rotate_decision` e callee=`wal_segment_is_empty` — o único par
restante do board cujos dois corpos já estavam extraídos no
`FlushKernel.lean` (o teste de produção
`wal_segment_is_empty_on_live_zero_is_not_ok` PINA que `Flush.lean`
dual-unfola os dois — o módulo canônico já existia).

Caller real (`db.rs::try_rotate_wal`):

1. `wal_rotate_decision(wal_pin_state())` → `KeepWal` pula;
2. recheck `commit_inflight` SOB a mutex do WAL (split Pebble-style —
   rotate não pode truncar frame recém-escrito);
3. `wal_segment_is_empty(w.position())` → segmento vazio pula (poll
   ocioso nunca reescreve MANIFEST);
4. senão `rotate_wal_now()`.

## O teorema (close, molde iff ∀)

`try_rotate_step_rotates_iff_pins_clear_segment_live` em
`formal/aeneas/lean/Flush.lean` (sorry 0, build verde):

```
∀ (s : WalPinState) (recheck_inflight : Bool) (pos : U64),
  (bind (wal_rotate_decision s) step) = ok true ↔
    (wal_rotate_decision s = ok RotateWal
      ∧ recheck_inflight = false
      ∧ wal_segment_is_empty pos = ok false)
```

onde `step` é a composição na ordem exata do caller (match na decisão;
if no recheck; bind no segmento). Propriedade nomeada sobre TODOS os
inputs: **o passo rotaciona exatamente quando a decisão do kernel diz
RotateWal, o recheck está idle e o segmento TEM dados** — o bloco de
rotação ociosa (segmento vazio) e o de corrida (recheck positivo) são
ambos pinados como não-disparo.

## Registro (mesmo commit)

- `scripts/formal/catalog.json`: par NOVO `wal_rotate_decision`
  (298→299) — flush_kernel.rs, twin_kind close, handler
  `try_rotate_wal`, as_is `wal_rotate_decision_as_is_ignore_pin`,
  dst_plant `wal_segment_is_empty_on_live_zero_is_not_ok`.
- `scripts/ratchet/close_proofs.tsv`: linha
  `close catalog:wal_rotate_decision try_rotate_step_rotates_iff_pins_clear_segment_live Flush.lean wal_rotate_decision`.
- `scripts/ratchet/proof_depth.tsv`: `floor_close 3→4`.
- `scripts/formal/residuals.json`: `proof_depth.close 4→5`
  (registrados 4 + 1 twin não-registrado).

## Nota de concorrência de sessões

No momento do commit a sessão paralela tinha DUAS linhas `count` em
voo nos mesmos TSVs (auto_flush_due, scan_guard; floor_count 4→6;
residuals count 6). O commit foi feito por staging linha-a-linha
(HEAD + apenas as minhas edições), sem tocar o voo dela — as linhas
dela permanecem não-commitadas no worktree para o commit dela.

## Lições de prova (Lean/Aeneas)

- `cases` numa VARIÁVEL ligada por `obtain` devora as hipóteses que a
  mencionam (reverte e não re-introduz) — usar `split at h` no
  match/if, que substitui o construtor mantendo as demais hipóteses.
- No `Flush.lean` o tipo é `Aeneas.Std.U64` (o open local `Aeneas.Std`
  faz `Std.U64` nu falhar — diferente do `GroupCommit.lean`, que abre
  `Aeneas` + `Aeneas.Std`).
- Prova final: `bind_ok_inv`/`bind_intro` privados (mesma forma dos
  outros módulos), `split at hm` duplo (match + if), `injection` +
  `simp` nos ramos impossíveis, `cases` escopado dentro de `have` para
  `e = false`/`recheck = false`.
