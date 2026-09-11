# RFC-0208 P2.1 (1/2) — atom `l28_tcp_left`: o mundo real TCP relata a saída do membro iff o disco diz

Data: 2026-09-11. Par `l28_tcp_left` (`catalog:l28_tcp_left`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_left_ok`,
chamado ao vivo por `tcp_node_disk_left_joint` no binário
`cluster_real`). Escada: cap_data_fate 86→85, floor_atom 45→46,
floor_extract 233→232, residuals atom 45→46 / extract 233→232 /
data_fate 86→85 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_left  l28_tcp_left_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_left_ok`).

## O que foi pago

Primeira promoção da banda l28 — o destino do reporte de saída de
membro no nó TCP real sobre TODOS os inputs (L28.lean,
`l28_tcp_left_ok_fate_iff`) — corpo pure-lift `ok left`:

```lean
theorem l28_tcp_left_ok_fate_iff :
    ∀ (left : Bool) (v : Bool),
      (l28_tcp_left_ok left = ok v) ↔
        ((v = true ∧ left = true)
          ∨ (v = false ∧ left = false))
```

O nó relata "membro saiu no disco" EXATAMENTE quando o disco diz:
sem remoção fantasma, sem remoção escondida. O mutante AS-IS
`ok true` reporta saída incondicionalmente — a mentira que a
planta TCP REAL (`l28_real_tcp_remove_member_left_on_disk`,
protocolo TCP de verdade entre nós, 235s) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=46 (floor 46), extract=232 (floor 232),
  data_fate=85≤85, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_remove_member_left_on_disk`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (235.94s).
