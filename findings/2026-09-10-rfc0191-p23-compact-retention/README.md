# RFC-0191 P2.3 (passo 16) — compact_retention graduado a atom (cap 115→114)

**Data:** 2026-09-10 · **Fire:** 774 · **Estado:** landed

## Alvo

O par `compact_retention` (F177/F20, `pedradb-core/src/
compact_kernel.rs`, entry `point_version_fate`, handler
`gc_snapshot_safe` em merge.rs) estava parado em `data_fate`. A
garantia: **uma versão de ponto cai exatamente quando existe versão
mais nova mantida E ela está no-ou-abaixo do oldest snapshot** — versão
visível ao snapshot mais velho nunca cai; sem cópia mais nova nunca
cai.

## Teorema (atom registrado)

`point_version_fate_drop_iff_newer_kept_at_or_below_oldest_snapshot`
(`Compact.lean`, ∀ `this_seq newer_kept_seq oldest_snapshot`):
`ok VersionFate.Drop` ↔ (∃ newer_seq, newer_kept_seq = some newer_seq ∧
newer_seq ≤ oldest_snapshot). 0 sorry.

### Lean lore (Fire 774)

- `split at h` num **match de Option NÃO destrói**: introduz variável
  fresca + EQUAÇÃO (`c : some p = some n✝`) — os bullets seguintes
  consomem as hipóteses erradas. Usar `cases newer_kept_seq` (substitui
  meta E hipóteses) primeiro.
- Depois de `cases ... | some p`, `simp at h` numa equação
  `(if p ≤ oldest then ok Drop else ok Keep) = ok Drop` resolve o ite
  SOZINHO e devolve `h : ↑p ≤ ↑oldest_snapshot` (o ramo else é
  construtor≠construtor, o simp descarta; sobra a condição) — witness
  direto `⟨p, rfl, h⟩`.
- Backward `rw [hp]; exact if_pos hc` — o `match some p` iota-reduz por
  defeq no `exact`.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 115 | **114** (`compact_retention` gradua) |
| `floor_atom` | 16 | **17** |
| `floor_extract` / residuals `extract` | 263 | **262** (recount) |
| residuals `proof_depth` | 263/2/16/17 | **262/2/17/17** |
| residuals `data_fate`/`single_artifact` | 115/291 | **114/291** |

## Evidência

- `lake build Compact` → verde, 0 sorry.
- `cargo test -p pedradb-core --lib point_version_fate_on_live_snapshot`
  → planta (no próprio kernel) verde.
- depth-floor GREEN (extract 262, atom 17/17, data_fate 114≤114);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
