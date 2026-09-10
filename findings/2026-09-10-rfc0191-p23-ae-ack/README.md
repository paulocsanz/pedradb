# RFC-0191 P2.3 (passo 13) — ae_ack graduado a atom (cap 118→117)

**Data:** 2026-09-10 · **Fire:** 771 · **Estado:** landed

## Alvo

O par `ae_ack` (F48, `pedradb-raft/src/ae_kernel.rs`, entry
`ae_ack_success`) estava parado em `data_fate`. A garantia: **um ack de
AppendEntries ok exatamente quando o log está limpo, OU está sujo e o
persist deu certo** — log sujo com persist falho nunca acka ok.

## Teorema (atom registrado)

`ae_ack_success_ok_iff_clean_or_dirty_persisted` (`Ae.lean`, ∀
`log_dirty persist_ok`): `ok true` ↔ (`log_dirty = false ∨
persist_ok = true`). 0 sorry.

### Lean lore (Fire 771)

- Extrato `if log_dirty then ok persist_ok else ok true` é o shape
  767 (o `then` carrega a variável): no backward, o PRIMEIRO bullet do
  `split` é `rw [hc]` e o segundo `rfl` — copiar do 768 (shape 768:
  `then ok true`) troca os ramos e falha com "rfl failed: ok persist_ok
  is not definitionally equal to ok true". Ler QUAL variável o then
  carrega antes de escolher a ordem dos bullets.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 118 | **117** (`ae_ack` gradua) |
| `floor_atom` | 13 | **14** |
| `floor_extract` / residuals `extract` | 266 | **265** (recount) |
| residuals `proof_depth` | 266/2/13/17 | **265/2/14/17** |
| residuals `data_fate`/`single_artifact` | 118/291 | **117/291** |

## Evidência

- `lake build Ae` → verde, 0 sorry.
- `cargo test -p pedradb-store --lib ae_ack_success_on_live_queued`
  → planta verde.
- depth-floor GREEN (extract 265, atom 14/14, data_fate 117≤117);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
