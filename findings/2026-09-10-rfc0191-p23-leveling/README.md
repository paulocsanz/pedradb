# RFC-0191 P2.3 passo 24 — `leveling` graduado a atom (Fire 782)

**Data:** 2026-09-10 · **Par:** `leveling` · **Kernel:** `crates/pedradb-core/src/leveling.rs` · **Finding:** F-leveling-sweep

## O que fechou

`level_target_bytes` — o alvo de tamanho por nível do leveled compaction —
tem agora um iff forall sobre o corpo Aeneas extraído
(`LevelingKernel.lean`, `aeneas_leveling.sh`):

```
level_target_bytes_ok_iff_zero_or_fanout_chain
  ∀ (level : U32) (l1_target : U64) (v : U64),
  (level_target_bytes level l1_target = ok v) ↔
    ((level = 0#u32 ∧ v = 0#u64)
     ∨ (¬(level = 0#u32)
        ∧ ∃ i e f,
            level - 1#u32 = ok i
            ∧ Ord.min i 18#u32 = ok e
            ∧ saturating_pow LEVEL_FANOUT e = ok f
            ∧ saturating_mul l1_target f = ok v))
```

A semântica honesta: nível 0 não tem alvo (valor 0); nível não-zero
computa `l1_target · FANOUT^min(level−1,18)` com aritmética saturante, e
o iff declara que o valor sai exatamente quando **cada passo do encadeamento
monádico é ok** e o `saturating_mul` final produz o valor — a cadeia ∃ é a
regra de computação do `do`/`bind`, não uma tautologia.

## Por que a forma ingênua seria UNSOUND

`= ok 0#u64 ↔ level = 0#u32` é **falsa**: com `l1_target = 0` e nível
≠ 0, `saturating_mul 0 _ = ok 0` — o valor 0 sem ser L0. O ∃-encadeado
(derivado do `ae_entry` passo 21) fecha a lacuna: cada degrau precisa ser
`ok` individualmente, e a desordem `¬(level = 0#u32)` no ramo direito
impede o sub-underflow `0#u32 − 1#u32` de fingir um `i`.

## Prova

Dois lemas privados genéricos sobre `Result`:

- `bind_ok_inv`: `bind x f = ok v → ∃ a, x = ok a ∧ f a = ok v`
  (`cases x` na variável livre — fail/div são absurdos).
- `bind_intro`: a recíproca, via `rw [hx]`.

Lore nova: **`rw` dentro de hipótese após `cases h : <termo opaco>`**
corrompe os invariantes de máquina-int (`decide (0 ≤ cMax …)` não
reduz em `instances` transparency — `simp` então "não faz progresso").
A rota limpa é `cases` sobre **variável livre** em lema auxiliar, nunca
`rw … at h` sobre o termo opaco. E `bind` é ambíguo com a notação
monádica — usar `Aeneas.Std.bind` qualificado.

## Números

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 107 | **106** |
| floor_atom | 24 | **25** |
| floor_extract | 255 | **254** |
| registered atoms | 24 | 25 |

## Verificação (mesma árvore, antes do commit)

- `lake build Leveling` — verde, **0 sorry** no módulo.
- `scripts/check_depth_floor.py` — GREEN (extract=254, atom=25,
  data_fate=106).
- `scripts/check_product_floor.py` — GREEN (promoted 4/4).
- Planta three-teeth: `cargo test -p pedradb-core --lib
  level_target_bytes` — `1 passed`
  (`leveling::tests::level_target_bytes_on_live_deep_level_is_not_ok`,
  a planta do expoente 18: nível 19 == nível 20).
- `pedra_formal.py`: `ok leveling: proof_depth=atom (2026-09-10)`;
  exit 1 herdado (146 FAILs da sessão paralela, nenhum nomeia este fire).

## Trampolim

`prepare_l0_compact` / `prepare_pushdown_compact` (`db.rs`) chamam o
kernel Rust que decide compactação por este alvo — a decisão do
trampolim casa com a do kernel provado.

## Próximo (783)

`compact_rewrites_sst_cf` — composição encode+key_in_cf_family
(opaca); depois `decode_cf_key`, `encode_cf_key`, `infer_sst_cf`.
Aritmética do cap: 5 itens na fila, cap precisa ≤100 — déficit 2 a
confrontar (re-scope honesto ou pagamentos extra dos 26 pares sem
`aeneas`).
