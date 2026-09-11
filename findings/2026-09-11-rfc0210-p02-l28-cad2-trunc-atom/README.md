# RFC-0210 P0.2 (1/4) — atom `l28_tcp_trunc`: disco sem index>commit iff o truncate persistiu

Data: 2026-09-11. Par `l28_tcp_trunc` (`catalog:l28_tcp_trunc`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_trunc_ok`).
Escada: cap_data_fate 80→79, floor_atom 51→52,
floor_extract 227→226, residuals atom 51→52 / extract 227→226 /
data_fate 80→79 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_trunc  l28_tcp_trunc_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_trunc_ok`).

## O que foi pago

Primeira promoção da cadência 2/4 — o destino do truncate de
recuperação sobre TODOS os inputs (L28.lean,
`l28_tcp_trunc_ok_fate_iff`) — corpo pure-lift `ok b`:

```lean
theorem l28_tcp_trunc_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_trunc_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

Depois de uma planta TCP REAL + morte de processo, o recover
truncate persiste de forma que o disco NÃO tem `index > commit`
numa réplica tirada de `ids` EXATAMENTE quando o truncate
persistiu. O mutante AS-IS `ok true` pula o persist do truncate da
réplica removida (o leftover 0131: só ids) — a mentira que a planta
TCP REAL (`l28_real_tcp_removed_truncate`) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=52 (floor 52), extract=226 (floor 226),
  data_fate=79≤79, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_truncate`
  (pedradb-store/tests/l28_real_tcp.rs) — verde.
