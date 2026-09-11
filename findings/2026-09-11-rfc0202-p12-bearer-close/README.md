# RFC-0202 P1.2 — quinto close registrado: `bearer_token_from_value`

Data: 2026-09-11. Escada: floor_close 4→5, residuals proof_depth.close
5→6 (5 registrados + 1 twin sem extração). Par `bearer`
(`crates/pedradb-http/src/auth_kernel.rs`, extração
AuthKernel.lean, chamado pelo handler `authorize` em
`crates/pedradb-http/src/lib.rs`).

## O que foi pago

Fate iff de 7 vias do output INTEIRO de `bearer_token_from_value` sobre
a cadeia de callees (Auth.lean,
`bearer_token_from_value_fate_iff`): trim → is_empty →
split_once_ws → `is_bearer_scheme` / `is_non_bearer_auth_scheme`. Cada
destino none/some do RHS pinha exatamente qual callee respondeu o quê:

1. valor trims vazio ⇒ `none`;
2. split `(scheme, rest)` com scheme bearer e token vazio ⇒ `none`;
3. split com scheme bearer e token não-vazio ⇒ `some tok`;
4. split com scheme NÃO-bearer ⇒ `none` (Basic/Digest/Negotiate/NTLM
   com credenciais não é token compartilhado — F150/F151);
5. sem split, valor É um scheme bearer ⇒ `none` (Authorization só com
   scheme, sem credenciais);
6. sem split, valor é outro scheme de auth ⇒ `none`;
7. sem split, valor não é scheme ⇒ `some v` (token cru).

Os dois gates de scheme são os callees pedra-locais extraídos
(`is_bearer_scheme`, `is_non_bearer_auth_scheme`, ambos catalogados); os
primitivos `core.str` (trim/is_empty/split_once_ws) seguem
axiomas-opacos do Aeneas — a ponte quantifica existencialmente sobre os
resultados deles (mesmo molde de hipóteses do P1.1, agora como ∃ do
iff). Direção →: inversão bind-a-bind (`bind_ok_inv` + `split at`);
direção ←: `simp [hipóteses]` normaliza o do-block e fecha cada folha.

## Queda medida do candidato do RFC

`group_validate` (passo N-way do lost-update, primeiro candidato do
P1.2) é extraído como `partial_fixpoint` — irredutível a defeq,
documentado no header de GroupCommit.lean. Queda para o próximo par do
board com corpo tratável, no padrão 0200 P1.2 (cadência não trava).

## Registro (mesmo commit)

- `close_proofs.tsv`: linha
  `close catalog:bearer bearer_token_from_value_fate_iff formal/aeneas/lean/Auth.lean bearer_token_from_value`
- `proof_depth.tsv`: floor_close 4→5
- `scripts/formal/residuals.json`: glue.proof_depth.close 5→6
- catálogo intocado (par de close não pede cirurgia — cf.
  occ_batch_plan, que mantém `data_fate` até hoje); cap_data_fate e
  floor_extract intactos (par `bearer` não tem data_fate)

## Verificação

- `lake build Auth` verde (1699 jobs), zero `sorry` em Auth.lean
- gates: depth-floor GREEN (registered close=5 floor 5, residuals
  close=6 == live 6/36, extract 242, data_fate 95≤95), product-floor
  GREEN, ledger GREEN 299/266/33
- `scripts/lean_extracts.sh --required`: ok (61 libs + 12 compose)
- planta DST `bearer_token_from_value_on_live_http_is_not_ok`
  (pedradb-http): 1 passed

## Lições de prova (soma às do P1.1)

- `ok some (scheme, rest)` NÃO elabora (`ok some` vira aplicação
  parcial): sempre `ok (some (scheme, rest))`.
- split de Bool-if entrega `¬(b = true)` no caso falso; para usar como
  equação em `rw`, converter primeiro com
  `simp only [Bool.not_eq_true] at h`.
- Padrão rintro monolítico com alternativas aninhadas em ⟨⟩ falha a
  parsear; `rintro ⟨v, hw, hbranch⟩` + `rcases` em estágios é o formato
  que compila.
