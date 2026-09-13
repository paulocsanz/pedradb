# RFC-0216 — Superfície de parse HTTP no degrau átomo: cl ×4 + fail_closed ×1 + form ×7 + path ×8

**Status:** draft
**Data:** 2026-09-12
**Autoria:** agente grind (round 9→10), sucessora direta do RFC-0215
(coroa de produto fechada, `**Status:** done` no HEAD `c130482e`)

## Contexto

O RFC-0215 fechou a coroa de produto no degrau átomo (spec ×4 + modelo
×4 + fate ×2 = 10 átomos; http ×6 = 6 átomos; composição
espinha→coroa `ComposeProductCrown.lean` citando a iff de `d1_modelo`)
e o sweep final 3× GREEN em worktree DENTRO de `software/`. A escada
medida ao vivo no HEAD `c130482e`: extract=120, close=6, atom=158,
count=7, `data_fate 0<=0` GREEN.

A medição ao vivo (`candidates.py` no HEAD) zerou TODOS os boards de
máquina: `unpaid_script 0/17`, `unpaid_compose 0/17`,
`unpaid_concurrency 0/5`, `unpaid_scale 0/3`, `unpaid_product 0`,
`unpaid_no_extract none`, buracos A/B/D/C vazios, `atom_to_close
none`. O cartoon restante (`cartoon_twin=4`) vive todo em Montanha —
**skip** até o usuário levantar (regra 13 do caminho). O
`leftover_next` caiu no fallback P1.5 (trampolim db.rs/concurrent.rs),
mas com `cap_data_fate` já em 0 e o trampolim congelado por non-goal
da rodada 9 (mesma regra vale aqui), o que resta pagável de máquina é
a ESCADA: 120 pares cujo melhor artefato Lean ainda é teorema de
extrato (valor concreto, sem ∀).

Dentre eles, o maior bloco coerente é a **superfície de parse HTTP** —
20 pares em 3 kernels do `crates/pedradb-http`:

- **cl ×4** — `cl_kernel.rs` (wrapper `Cl.lean`):
  `content_length` (`keep_body_without_cl`), `invalid_cl_zero`
  (`invalid_cl_as_zero`), `cl_repeat_conflict`
  (`content_length_repeat_ok`), `short_body_vs_cl`
  (`short_body_vs_cl_is_error`) — os gates que decidem se um corpo é
  admitido, rejeitado ou truncado;
- **fail_closed ×1** — `fail_closed.rs` (wrapper `FailClosed.lean`):
  `fail_closed` (`parse_error_writes_status`) — o veredito de erro
  fail-closed do parse inteiro;
- **form ×7** — `form_kernel.rs` (wrapper `Form.lean`): `form_plus`
  (`form_decode`), `form_plus_byte`, `plus_before_percent`,
  `from_hex`, `query_values_conflict`, `query_u64_conflict`,
  `query_part_is_bare_name` — decodificação de form/query e os
  conflitos que rejeitam;
- **path ×8** — `path_kernel.rs` (wrapper `Path.lean`):
  `origin_path` (`origin_form_path`), `path_after_authority`,
  `strip_http_authority`, `request_target_authority`,
  `host_authority_mismatch`, `split_host_port`, `strip_uri_fragment`,
  `strip_authority_for_routing` — a autoridade e o caminho que decidem
  o roteamento de TODO request vivo.

O salto seL4 deste RFC: parse é a superfície de ataque clássica de um
kernel de serviço verificado — fechar os 20 gates em iff-∀ deixa a
família http do catálogo **27/27 em degrau átomo** (7 do RFC-0215 +
`bearer` do 0214 + 20 daqui): nenhum gate do plano de request
(admissão de corpo, veredito de erro, form/query, autoridade/rota)
decidido por teste em vez de teorema, e o degrau extrato cruza a
barreira redonda de 100.

## Meta mensurável

Escada final (se os 20 pousarem): `floor_atom 158→178`,
`floor_extract 120→100`, close=6, count=7 e `cap_data_fate 0<=0`
imutáveis. Contagem autoritativa: o gate `check_depth_floor.py` no
HEAD de cada promoção. Cadência: 1 promoção = 1 commit (teorema iff
no wrapper, planta DST verde ANTES do commit, gate GREEN no commit).

## Fatias

1. **P0.1** cl ×4 (`Cl.lean`): `keep_body_without_cl`,
   `invalid_cl_as_zero`, `content_length_repeat_ok`,
   `short_body_vs_cl_is_error` — iff-∀ sobre os corpos extraídos dos
   gates de Content-Length — floor_atom 158→162, floor_extract
   120→116 — status: `done` (2026-09-13; floor_atom 162,
   floor_extract 116)

2. **P0.2** fail_closed ×1 (`FailClosed.lean`):
   `parse_error_writes_status` — a perna de erro do parse em átomo;
   P0 completo = todo veredito de admissão de request (corpo + erro)
   em teorema — floor_atom 162→163, floor_extract 116→115 —
   status: `done` (2026-09-13; floor_atom 163, floor_extract 115)

3. **P1.1** form ×4 (`Form.lean`): `form_decode`, `form_plus_byte`,
   `plus_before_percent`, `from_hex` — a decodificação byte a byte e
   o hex — floor_atom 163→167, floor_extract 115→111 — status: `done`
   (2026-09-13; 4/4, o `form_decode` fechou o primeiro loop da família
   via cadeia DecodeFate por indução em combustível)

4. **P1.2** form ×3 (`Form.lean`): `query_values_conflict`,
   `query_u64_conflict`, `query_part_is_bare_name` — os conflitos de
   query que rejeitam — floor_atom 167→170, floor_extract 111→108 —
   status: `done` (2026-09-13; 3/3, o `query_values_conflict`
   fechou o segundo loop da família via cadeia ValuesFate)

5. **P2.1** path ×8 (`Path.lean`): `origin_form_path`,
   `path_after_authority`, `strip_http_authority`,
   `request_target_authority`, `host_authority_mismatch`,
   `split_host_port`, `strip_uri_fragment`,
   `strip_authority_for_routing` — autoridade e rota em teorema;
   família http 27/27 em átomo — floor_atom 170→178, floor_extract
   108→100 — status: `todo` (4/8: `strip_authority_for_routing`,
   `strip_uri_fragment`, `path_after_authority`,
   `strip_http_authority` feitos 2026-09-13)

6. **P2.2** sweep final em worktree DENTRO de `software/` (gates 3×
   GREEN + campaign ok + extracts ok + sorry 0, capturas
   `{SCRATCH}/r0216_sweep_*`) + nota datada `EXTRACT.md` + flip
   `**Status:** done` — status: `todo`

## Vereditos / riscos

- Se algum corpo extraído não comportar a iff (recusa MEDIDA e
  nomeada), veredito datado em findings + `EXTRACT.md` — sem gate
  inventado; o alvo da meta ajusta-se ao caminho medido (mesma regra
  dos RFC-0214/0215, que fecharam "nenhum recusado").
- `promote_atom.py` (round 8/9, `{SCRATCH}`) segue válido: TSV com
  tabs reais, `floor_atom +1 / floor_extract −1`, `atom_reason`
  datado, `residuals.json` — nunca `json.dump` no catálogo vivo.
- Moldes Lean do round 9 valem para loops de parse: o caso
  `let (k, v) := (k, v)` elaborado reduz só por `conv => lhs; whnf`;
  `∨` é associativo à direita em Lean 4 (rcases e construtores
  anônimos aninhados explicitamente).

## Não-metas

- NÃO despejar `db.rs` / `concurrent.rs` (trampolim esvazia-se, não
  vaza; `leftover_next` P1.5 fica para o usuário levantar).
- NÃO tocar `crates/montanha-fdb-recipes/**` (cartoon em skip até o
  usuário levantar).
- NÃO flipar `forall_schedules` / `media_durable` /
  `lock_interleavings` admitted; campanha ≠ ∀π segue TCB.
- Disk/`fdatasync` Ok não é mídia (RFC-0078); `never_floor` imutável
  (R-cpu R-rustc R-verus R-crc R-deps).

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | cl ×4 átomo (Content-Length) | done | `keep_body_without_cl_fate_iff`/`invalid_cl_as_zero_fate_iff`/`content_length_repeat_ok_fate_iff`/`short_body_vs_cl_is_error_fate_iff` | 2026-09-13 |
| P0.2 | p0 | fail_closed ×1 átomo (veredito de erro) | done | `parse_error_writes_status_fate_iff` | 2026-09-13 |
| P1.1 | p1 | form ×4 átomo (decode/hex) | done | `form_plus_byte_fate_iff`/`plus_before_percent_fate_iff`/`from_hex_fate_iff`/`form_decode_fate_iff` | 2026-09-13 |
| P1.2 | p1 | form ×3 átomo (conflitos de query) | done | `query_u64_conflict_fate_iff`/`query_part_is_bare_name_fate_iff`/`query_values_conflict_fate_iff` | 2026-09-13 |
| P2.1 | p2 | path ×8 átomo (autoridade/rota; http 27/27) | todo | — | 2026-09-12 |
| P2.2 | p2 | Sweep final + nota EXTRACT.md + flip done | todo | — | 2026-09-12 |

## Critérios de aceite

- **P0.1–P2.1 (cada átomo)**: teorema iff-∀ no wrapper com `lake
  build` verde; `promote_atom.py` (TSV + floors + `atom_reason` +
  residuals); `python3 scripts/check_depth_floor.py` GREEN; planta
  DST isolada verde com exit checado; **1 teorema/commit** (`git show
  HEAD -- <lean> | grep -c "^+theorem"` == 1); linha da RFC flipada
  no mesmo commit; findings README da fatia.
- **P2.2**: worktree destacado DENTRO de `software/`: depth-floor
  GREEN (atom=178/extract=100 se os 20 pousarem), inventory
  terminal, twin-contracts bound, `test_proof_vs_campaign.py` ok,
  extracts ok, sorry 0 nos wrappers da rodada; capturas
  `{SCRATCH}/r0216_sweep_*`; nota datada em `EXTRACT.md`;
  `**Status:** done` no mesmo commit.
