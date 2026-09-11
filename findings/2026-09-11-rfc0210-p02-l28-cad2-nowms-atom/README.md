# RFC-0210 P0.2 (4/4) — atom `l28_tcp_nowms`: now_ms persistido iff o persist aconteceu

Data: 2026-09-11. Par `l28_tcp_nowms` (`catalog:l28_tcp_nowms`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_nowms_ok`).
Escada: cap_data_fate 77→76, floor_atom 54→55,
floor_extract 224→223, residuals atom 54→55 / extract 224→223 /
data_fate 77→76 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_nowms  l28_tcp_nowms_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_nowms_ok`).

## O que foi pago

Quarta e última promoção da cadência 2/4 — o destino do persist de
`now_ms` sobre TODOS os inputs (L28.lean,
`l28_tcp_nowms_ok_fate_iff`) — corpo pure-lift `ok b`:

```lean
theorem l28_tcp_nowms_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_nowms_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

Depois de uma planta TCP REAL + morte de processo, o `now_ms` é
persistido numa réplica tirada de `ids` EXATAMENTE quando o persist
aconteceu. O mutante AS-IS `ok true` pula o persist (o leftover
0134: só ids) — a mentira que a planta TCP REAL
(`l28_real_tcp_removed_now_ms`) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=55 (floor 55), extract=223 (floor 223),
  data_fate=76≤76, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_now_ms`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (346.54s).

## Cadência 2/4 fechada nos números exatos do RFC

cap_data_fate 80→76, floor_atom 51→55, floor_extract 227→223 —
4 atoms, 4 commits (4b44881b, df80dd90, a4ef110f, este), plantas
TCP REAIS 4/4 verdes (367s/364s/234s/347s). P0 inteiro fechado:
8 atoms, 8 commits, cap 84→76.
