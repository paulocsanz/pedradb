# RFC-0210 P0.2 (2/4) — atom `l28_tcp_odrop`: órfãos além do novo hi caem iff caíram

Data: 2026-09-11. Par `l28_tcp_odrop` (`catalog:l28_tcp_odrop`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_odrop_ok`).
Escada: cap_data_fate 79→78, floor_atom 52→53,
floor_extract 226→225, residuals atom 52→53 / extract 226→225 /
data_fate 79→78 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_odrop  l28_tcp_odrop_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_odrop_ok`).

## O que foi pago

Segunda promoção da cadência 2/4 — o destino da queda dos órfãos
sobre TODOS os inputs (L28.lean, `l28_tcp_odrop_ok_fate_iff`) —
corpo pure-lift `ok b`:

```lean
theorem l28_tcp_odrop_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_odrop_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

Depois de uma planta TCP REAL + morte de processo, o recover
truncate APAGA as linhas `log_entry_key` além do novo hi numa
réplica tirada de `ids` EXATAMENTE quando os órfãos caíram. O
mutante AS-IS `ok true` pula a queda do segmento órfão (o leftover
0132: só watermark) — a mentira que a planta TCP REAL
(`l28_real_tcp_removed_orphan_drop`) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=53 (floor 53), extract=225 (floor 225),
  data_fate=78≤78, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_orphan_drop`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (363.88s).
