# RFC-0210 P0.1 (1/4) — atom `l28_tcp_dterm`: o rollback durável do termo iff ele segurou

Data: 2026-09-11. Par `l28_tcp_dterm` (`catalog:l28_tcp_dterm`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_dterm_ok`).
Escada: cap_data_fate 84→83, floor_atom 47→48,
floor_extract 231→230, residuals atom 47→48 / extract 231→230 /
data_fate 84→83 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_dterm  l28_tcp_dterm_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_dterm_ok`).

## O que foi pago

Primeira promoção da cadência 1/4 do bloco l28 (0210) — o destino
do rollback de termo sobre TODOS os inputs (L28.lean,
`l28_tcp_dterm_ok_fate_iff`) — corpo pure-lift `ok b`:

```lean
theorem l28_tcp_dterm_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_dterm_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

No diretório REAL da réplica removida, um RequestVote de termo
novo cujo persist de hard state FALHA rola o termo de volta
EXATAMENTE quando o rollback segurou: reply, memória e disco
ficam no termo anterior (F125/F127). O mutante AS-IS `ok true`
mantém a elevação não-durável (termo de memória acima do hard
state de disco) — a mentira que a planta TCP REAL
(`l28_real_tcp_removed_durable_term`) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=48 (floor 48), extract=230 (floor 230),
  data_fate=83≤83, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_durable_term`
  (pedradb-store/tests/l28_real_tcp.rs) — verde.
