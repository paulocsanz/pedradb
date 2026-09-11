# RFC-0191 P2.3 passo 30 — `dir_sync_required` a atom; P2.3 FECHA no número

Fire 788. Segunda e última descida do déficit-2. Com este par:
**cap_data_fate 100 ≤ 100 e floor_atom 31 ≥ 8 — o alvo numérico do P2.3
está fechado** (30 passos pagos, 30 commitados um-par-por-commit).

## Par

- `dir_sync_required` (`crates/pedradb-core/src/write_admission_kernel.rs`
  L340), extrato `WriteAdmissionKernel.lean` L390, pago já antes pelo script
  verus (`scripts/verus_write_admission.sh`).
- Corpo extraído: `ok sync` — o lift puro; `dir_sync_required_as_is`
  devolve `ok false` (dente: o mutante mente que nunca precisa de fsync).

## Teorema (WriteAdmission.lean, 0 sorry, primeira build)

`dir_sync_required_ok_iff_sync : ∀ (sync v : Bool),
  (dir_sync_required sync = ok v) ↔ (sync = v)`

Sem passo monádico, sem comparação opaca: a iff É a regra de computação do
do-block inteiro. Semântica paga: rename/create só é seguido de fsync de
diretório quando o sync das open-options está ligado — o mutante AS-IS
abre a janela de crash em que o arquivo de dados aparece sem o dirent
durável.

## Números

| métrica | antes | depois |
|---|---|---|
| floor_atom | 30 | 31 |
| floor_extract | 249 | 248 |
| cap_data_fate | 101 | **100** |
| df live | 101 | 100 |

Gates GREEN (depth-floor extract=248/atom=31/df 100≤100; product-floor).
Plant `dir_sync_required_on_live_sync_is_not_ok` 1 passed. Lint:
`ok dir_sync_required: proof_depth=atom (2026-09-10)`, counts
`extract=248 close=2 atom=31 model=17`; 153 FAILs — conjunto IDÊNTICO ao
pf_787 (comm vazio), todos herdados da sessão paralela.

## Fecho do P2.3

- Alvo: cap ≤100 ✓ (100) e floor_atom ≥8 ✓ (31).
- Cadência honrada: cada descida de cap no MESMO commit do atom e do
  recount −1 do extract (passos 4–30 + `visible_at`).
- Déficit-2 resolvido pagando 2 pares da fila write_admission
  (`batch_is_empty`, `dir_sync_required`) — auditados como já-extraídos,
  sem trabalho novo de pipeline.
