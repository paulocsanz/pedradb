# RFC-0210 P0.1 (3/4) — atom `l28_tcp_apply`: recover apply fecha commit>applied iff aplicou

Data: 2026-09-11. Par `l28_tcp_apply` (`catalog:l28_tcp_apply`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_apply_ok`).
Escada: cap_data_fate 82→81, floor_atom 49→50,
floor_extract 229→228, residuals atom 49→50 / extract 229→228 /
data_fate 82→81 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_apply  l28_tcp_apply_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_apply_ok`).

## O que foi pago

Terceira promoção da cadência 1/4 — o destino do recover apply
sobre TODOS os inputs (L28.lean, `l28_tcp_apply_ok_fate_iff`) —
corpo pure-lift `ok b`:

```lean
theorem l28_tcp_apply_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_apply_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

Depois de uma planta TCP REAL + morte de processo, o recover apply
fecha `commit > applied` EXATAMENTE quando a recuperação aplicou
(construtor TCP de produção). O mutante AS-IS `ok true` pula o
recover apply (o leftover 0129: o joint commitado fica em C-old) —
a mentira que a planta TCP REAL (`l28_real_tcp_recover_apply`)
refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=50 (floor 50), extract=228 (floor 228),
  data_fate=81≤81, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_recover_apply`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (381.46s).
