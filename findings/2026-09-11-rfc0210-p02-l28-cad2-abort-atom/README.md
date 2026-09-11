# RFC-0210 P0.2 (3/4) — atom `l28_tcp_abort`: intents 2PC remanescentes caem iff o abort apagou

Data: 2026-09-11. Par `l28_tcp_abort` (`catalog:l28_tcp_abort`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_abort_ok`).
Escada: cap_data_fate 78→77, floor_atom 53→54,
floor_extract 225→224, residuals atom 53→54 / extract 225→224 /
data_fate 78→77 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_abort  l28_tcp_abort_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_abort_ok`).

## O que foi pago

Terceira promoção da cadência 2/4 — o destino do abort de
recuperação sobre TODOS os inputs (L28.lean,
`l28_tcp_abort_ok_fate_iff`) — corpo pure-lift `ok b`:

```lean
theorem l28_tcp_abort_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_abort_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

Depois de uma planta TCP REAL + morte de processo, o recover abort
APAGA os intents 2PC remanescentes numa réplica tirada de `ids`
EXATAMENTE quando o abort os apagou. O mutante AS-IS `ok true` pula
o abort do leftover (o leftover 0133: só ids) — a mentira que a
planta TCP REAL (`l28_real_tcp_removed_abort`) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=54 (floor 54), extract=224 (floor 224),
  data_fate=77≤77, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_abort`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (233.85s).
