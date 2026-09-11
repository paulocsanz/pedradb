# RFC-0210 P0.1 (4/4) — atom `l28_tcp_napply`: recover apply na réplica removida iff aplicou

Data: 2026-09-11. Par `l28_tcp_napply` (`catalog:l28_tcp_napply`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_napply_ok`).
Escada: cap_data_fate 81→80, floor_atom 50→51,
floor_extract 228→227, residuals atom 50→51 / extract 228→227 /
data_fate 81→80 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_napply  l28_tcp_napply_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_napply_ok`).

## O que foi pago

Quarta e última promoção da cadência 1/4 — o destino do recover
apply na réplica removida sobre TODOS os inputs (L28.lean,
`l28_tcp_napply_ok_fate_iff`) — corpo pure-lift `ok b`:

```lean
theorem l28_tcp_napply_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_napply_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

Depois de uma planta TCP REAL + morte de processo, o recover apply
fecha `commit > applied` numa réplica JÁ TIRADA de `ids` EXATAMENTE
quando a recuperação aplicou. O mutante AS-IS `ok true` pula o
recover apply da réplica removida (o leftover 0130: só ids) — a
mentira que a planta TCP REAL
(`l28_real_tcp_removed_recover_apply`) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=51 (floor 51), extract=227 (floor 227),
  data_fate=80≤80, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_recover_apply`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (390.71s).

## Cadência 1/4 fechada nos números exatos do RFC

cap_data_fate 84→80, floor_atom 47→51, floor_extract 231→227 —
4 atoms, 4 commits (9a0869ee, d6ea1e8b, 3d00e259, este), plantas
TCP REAIS 4/4 verdes (381s/381s/381s/391s, rodadas em paralelo).
