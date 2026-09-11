# RFC-0208 P2.1 (2/2) — atom `l28_tcp_hw`: o high-water do nó TCP real sobrevive à remoção iff o inventário commitado foi mantido

Data: 2026-09-11. Par `l28_tcp_hw` (`catalog:l28_tcp_hw`, kernel
`crates/pedradb-store/src/l28.rs`, entry `l28_tcp_hw_ok`, chamado ao
vivo por `tcp_node_disk_high_water` no binário `cluster_real`).
Escada: cap_data_fate 85→84, floor_atom 46→47,
floor_extract 232→231, residuals atom 46→47 / extract 232→231 /
data_fate 85→84 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_hw  l28_tcp_hw_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_hw_ok`).

## O que foi pago

Segunda e última promoção da banda l28 do 0208 — o destino do
high-water do nó TCP real sobre TODOS os inputs (L28.lean,
`l28_tcp_hw_ok_fate_iff`) — corpo pure-lift `ok kept`:

```lean
theorem l28_tcp_hw_ok_fate_iff :
    ∀ (kept : Bool) (v : Bool),
      (l28_tcp_hw_ok kept = ok v) ↔
        ((v = true ∧ kept = true)
          ∨ (v = false ∧ kept = false))
```

Depois de uma remoção, o high-water do nó move EXATAMENTE quando o
inventário commitado foi mantido: o progresso durável sobreviveu à
remoção — nem high-water fantasma, nem progresso escondido. O
mutante AS-IS `ok true` alega que o high-water SEMPRE moveu — a
mentira que a planta TCP REAL
(`l28_real_tcp_high_water_after_remove`, protocolo TCP de verdade
entre nós, 232s) refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=47 (floor 47), extract=231 (floor 231),
  data_fate=84≤84, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_high_water_after_remove`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (232.11s).

## Banda fechada; o resto do bloco l28 tem plano datado

P2.1 fecha em cap 86→84 nos números exatos do RFC. Os 29 pares l28
`data_fate` restantes têm plano datado em
`formal/aeneas/EXTRACT.md` (22 pure-lifts extraíveis com planta
real, 7 fantasmas de catálogo sem `fn` viva — conserto de catálogo,
não extração).
