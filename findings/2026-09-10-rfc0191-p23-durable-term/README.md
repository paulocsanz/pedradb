# RFC-0191 P2.3 (passo 8) — durable_term graduado a atom (cap 123→122)

**Data:** 2026-09-10 · **Fire:** 766 · **Estado:** landed

## Alvo

O par `durable_term` (F125/F127, `pedradb-raft/src/vote_kernel.rs`,
entry `durable_term_if_newer`) estava parado em `data_fate`. A product
guarantee (RFC-0158): **o termo sobe durável ou volta** — `Raised`
exatamente quando o termo que chega é estritamente novo E o persist deu
`Ok`; persist falho restaura o termo velho (`Restored`), termo velho/igual
mantém (`Keep`). Primeira graduação fora do store (crate raft).

## Teorema (atom registrado)

`durable_term_if_newer_raised_iff_newer_and_persisted` (`Vote.lean`,
∀ `current_term incoming_term persist`): plano = `Raised` ↔
(`incoming_term > current_term` ∧ `persist = PersistOutcome.Ok`).
0 sorry, 3 iterações de tactics.

### Lean lore (Fire 766)

- Comparação U64 no extrato elabora como **Prop** (`incoming > current`
  direto, sem `= true`): escrever `(x > y) = true` no enunciado dá
  `(x>y) = (true = true)` e quebra `if_pos`. Copiar a forma nua.
- `split at h` em `match persist`: o discriminante é SUBSTITUÍDO (o
  `next c` traz o valor, não uma equação) — provar a meta com `rfl`,
  não `exact c`; contradizer hipótese via `absurd hp (by simp)`.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 123 | **122** (`durable_term` gradua) |
| `floor_atom` | 8 | **9** (meta 8 superada) |
| `floor_extract` / residuals `extract` | 271 | **270** (recount) |
| residuals `proof_depth` | 271/2/8/17 | **270/2/9/17** |
| residuals `data_fate`/`single_artifact` | 123/291 | **122/291** |

## Evidência

- `lake build Vote` → verde, 0 sorry.
- `cargo test -p pedradb-store --lib durable_term` → 4 testes do
  kernel verde + planta
  `three_teeth_queued::durable_term_rollback_on_live_queued_is_not_ok`
  verde.
- depth-floor GREEN (extract 270, atom 9/9, data_fate 122≤122);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
