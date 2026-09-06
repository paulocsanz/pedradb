# RFC: refinamento à classe seL4 (close + extract; TCB inalterado)

**Status:** in-progress
**Updated:** 2026-09-06
**Parents:** [0061](0061-residuals-sel4-ironfleet.md), [0166](0166-prova-de-fato-refinamento-propriedades.md), [0151](0151-three-teeth-as-is-verus-dst.md), [0053](0053-ironfleet-years.md)

**Residual:** never_floor (`R-cpu` `R-rustc` `R-verus` `R-crc` `R-deps` `R-extract`) **não muda**. Glue `db.rs` não vira kernel. L28/PCT continuam campanha.

**Frase permitida no fim deste RFC:** cada decisão de produção que um handler chama é um gémeo **close** (`exec == spec` dessa fn), e cada close está extraído por Aeneas do **ficheiro de produção** (sha256 pinado, Lean sem `sorry`) — a mesma *classe de garantia* que seL4 pagou no C do microkernel, relativa ao TCB publicado.

**Frases recusadas:** “somos seL4”, “sem bugs”, “garantia total”, “o rustc/Z3/CPU/CRC/fsync está provado”, “extraímos `db.rs`”.

## Background

- RFC-0061 já respondeu **não** à pergunta “Pedra é tão robusto quanto seL4?”. seL4 escreveu o kernel no prover (~10 kLOC C, ~200 kLOC Isabelle). Pedra extrai **decisões** do Rust de produção. O TCB (compilador, hardware, prover) é a mesma *classe* nos dois lados — seL4 também não provou o gcc.
- RFC-0166 fechou D1/R1/T1/C1 como **modelos** + dentes + plantas. Isso é propriedade nomeada, ainda não é “o C que corre”.
- Freeze de hoje (HEAD, 2026-09-06): ~265 pares, ~216 close / **26 atom** / ~22 model; ~10 kernels Aeneas (`vote`, `ae`, `commit`, `apply`, `bloom`, `isolated`, `group_commit`, `reopen`, `wal_recover`, `probe_order`); `db_rs_extracted=false`; handler_loc ≫ kernel_loc.
- Gémeo `atom` = o Verus prova um pedaço (`bump_non_ff`, `pick_l0_to_l1_model`) e o handler chama outra fn (`prefix_exclusive_end`, `pick_l0_to_l1`). seL4 não fez isso: o C extraído *é* a fn.
- 2026-09-06: par `prefix` passou de atom→close (`4936ad94`) — o loop que `scan_prefix` chama, não só o byte. Esse é o movimento; faltam 26.
- Aeneas pinado (`charon 0.1.232` / `aeneas daa85d7`) traduz o ficheiro de produção. Iterator/closure ainda rebentam (RFC-0164 P2.1 mediu e recusou re-pin). Close com `while` + `match` é o que o extract aceita.

## Problems This Solves

- **Problem:** “nível seL4” não tem um mapa de fatias — só a recusa do 0061. Recusar o slogan sem um caminho é o que parece preguiça.
- **Problem:** 26 pares `atom` deixam o handler a chamar uma fn que o gémeo **não** prova.
- **Problem:** a maioria dos close ainda é twin heurístico (tokens ⊆), não extract do ficheiro de produção.
- **Problem:** D1/R1/T1/C1 do 0166 vivem em kernels-modelo; o corolário não está amarrado à fn close que o `Db`/`Store` chama.
- **Problem:** `verificacao-next` esgota D/C/B/E/F e pára. O próximo passo de refinamento (atom→close) não está no ranker.

## Proposed Solution

Três degraus, cada um fail-closed no lint, um par por turno:

1. **Close.** Toda `entry` de catálogo que um handler chama tem exec fn homónima no twin, `exec == spec`, tokens cobertos. Atom só com `atom_reason` datado. Model só com domínio stand-in declarado.
2. **Extract.** Todo close cujo corpo o Charon traduz vira crate Aeneas com `SOURCE.*` sha256 do ficheiro de produção + Lean sem `sorry` (padrão Isolated/Vote).
3. **Corolário.** D1/R1/T1/C1 restam como propriedades; as hipóteses passam a ser as fns close de produção, não só o modelo 0166.

Never_floor fica never. Encolher glue = **tirar** `if` de `db.rs` para um kernel nomeado (já o padrão), nunca promover `db.rs` a kernel.

## Delivery slices (mandatory)

### P0 — must ship first (o degrau fica visível e o ranker aponta-o)

- [x] **P0.1** Freeze `proof_depth`: cada par do catálogo é `atom` | `close` | `extract` (extract = close + stamp Aeneas no `EXTRACT.md` / `SOURCE.*`). `--lint` falha par novo `atom` sem `atom_reason`. O board do `candidates.py` lista os atom restantes — status: `done`
- [x] **P0.2** `verificacao-next` ranqueia **atom→close de um par data_fate** acima de F/E quando D/C/B/A estão vazios (um par por turno; nunca 26) — status: `done`
- [x] **P0.3** Extract Aeneas de `crates/pedradb-core/src/prefix.rs` (já close): stamp `SOURCE.prefix`, Lean `prefix_exclusive_end_matches_spec` sem `sorry`, dente as-is no extract — status: `done`

### P1 — next wave (fechar os atom, um grupo por fatia)

Ordem: data_fate vivo em `db.rs` primeiro, HTTP/FDB depois. Uma fatia = um grupo, não o grupo inteiro se não couber numa sessão.

- [x] **P1.1** `leveling_pick` + `leveling_pushdown`: close de `pick_l0_to_l1` / `pick_pushdown` (hoje atom `*_model` em u64; o `Db::prepare_*` chama a fn `Vec<u8>`) — status: `done`
- [x] **P1.2** Família HTTP path/URI (`origin_path`, `path_after_authority`, `strip_http_authority`, `request_target_authority`, `host_authority_mismatch`, `split_host_port`, `strip_uri_fragment`) — um par close por PR se o grupo não couber — status: `done`
- [x] **P1.3** Família HTTP auth/form (`bearer`, `is_bearer_scheme`, `is_non_bearer_auth_scheme`, `normalize_http_method`, `authorization_matches`, `form_plus`, `query_values_conflict`, `query_part_is_bare_name`) — status: `done`
- [x] **P1.4** FDB pack (`children`, `fields`, `fields_suffix`, `fields_decode`, `fields_pair_nul`, `index_val`, `exact_children`) — status: `done`
- [x] **P1.5** Restantes (`isolated` child-byte se ainda atom, `world_trajectory_fold`) — status: `done`
- [x] **P1.6** `--lint` recusa `atom` sem `atom_reason` *e* recusa `atom_reason` mais velho que 30 dias sem RFC filho — status: `done`

### P2 — later (extract + corolário + glue que encolhe)

- [x] **P2.1** Extract Aeneas de cada close cujo Charon traduz (fila: prefix se P0.3 verde; depois vote/ae já extraídos como baseline; depois um close novo por turno). Re-pin de Aeneas só se o extract **alargar** o conjunto sem `sorry` (RFC-0164 P2.1 recusou re-pin inútil) — status: `done` (prefix + write_admission; restantes close na fila, um por turno)
- [x] **P2.2** Model→close nos pares `data_fate` de domínio stand-in que o motor chama com os tipos de produção (`scan_guard`, `cf_family` / `encode_cf_key` / `infer_sst_cf` se ainda model, `wait_for_deadlock`) — um par por turno — status: `done`
- [x] **P2.3** D1/R1/T1/C1 do 0166 reenunciados como corolários das fns **close** de produção (Inv-WAL/Inv-LSM passam a citar `prefix_exclusive_end` / `pick_l0_to_l1` / `write_ack` close, não só o modelo) — status: `done`
- [x] **P2.4** Inventário dos `if` de destino de dados ainda em `db.rs` sem `entry` de catálogo; cada fatia extrai **um** para kernel (padrão 0151 flush-publish / 0164 probe-order). `glue.db_rs_extracted` permanece `false` — status: `done` (`write_admission_idle`; inventário em `findings/2026-09-06-rfc0170-db-rs-ifs/`)
- [x] **P2.5** Relatório vivo: contagem close/atom/model/extract + rácio prova:impl no `residuals.json` / board. never_floor inalterado — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | freeze proof_depth + atom_reason | done | this RFC | 2026-09-06 |
| P0.2 | p0 | ranker atom→close após D/C/B/A | done | this RFC | 2026-09-06 |
| P0.3 | p0 | Aeneas extract de prefix.rs | done | this RFC | 2026-09-06 |
| P1.1 | p1 | close leveling_pick / pushdown | done | this RFC | 2026-09-06 |
| P1.2 | p1 | close HTTP path/URI atoms | done | this RFC | 2026-09-06 |
| P1.3 | p1 | close HTTP auth/form atoms | done | this RFC | 2026-09-06 |
| P1.4 | p1 | close FDB pack atoms | done | this RFC | 2026-09-06 |
| P1.5 | p1 | close isolated / trajectory atoms | done | this RFC | 2026-09-06 |
| P1.6 | p1 | atom_reason expira | done | this RFC | 2026-09-06 |
| P2.1 | p2 | Aeneas extract dos close traduzíveis | done | prefix + write_admission; fila no EXTRACT.md | 2026-09-06 |
| P2.2 | p2 | model→close data_fate | done | this RFC | 2026-09-06 |
| P2.3 | p2 | D1/R1/T1/C1 sobre fns close | done | this RFC | 2026-09-06 |
| P2.4 | p2 | um `if` de db.rs → kernel por fatia | done | write_admission_idle | 2026-09-06 |
| P2.5 | p2 | relatório close/atom/extract | done | glue.proof_depth | 2026-09-06 |

Prelude (não é fatia deste RFC): `prefix` atom→close em `4936ad94` (2026-09-06).

## Acceptance Criteria

- **Tests**
  - P0.1: `python3 scripts/formal/pedra_formal.py --lint` falha um par `atom` sem `atom_reason` (teste de mutação no estilo `test_twin_mutation.py`). Board imprime a lista.
  - P0.2: `candidates.py` imprime uma linha `atom_to_close <id> <entry>` quando D/C/B/A estão vazios e resta atom data_fate.
  - P0.3: `./scripts/aeneas_prefix.sh --required` + `./scripts/lean_prefix.sh --required` verdes; `SOURCE.prefix` muda se `prefix.rs` mudar; Lean sem `sorry`.
  - Cada P1: Verus do par `N verified, 0 errors` (2×), `--lint` `close <entry> tokens covered`, `cargo test` da planta nomeada verde.
  - P2.3: os pares 0166 `d1_modelo` / `r1_modelo` / `t1_modelo` / `c1_modelo` citam no `ensures` uma fn close de produção (não só o modelo).
- **Telemetry / Analytics:** none — provas; o sinal é exit code do job `proof-check` e o stamp Aeneas.
- **Documentation:** este RFC actualizado no mesmo commit do código; linha no `docs/status.md`; `formal/aeneas/EXTRACT.md` em cada extract.
- **Screenshots:** backend-only.

## Out of scope

- Apagar `never_floor` (CompCert / prova do rustc / colisão CRC / extract total).
- Extrair `db.rs` como kernel (L46, R-extract, RFC-0061 P1.3).
- Reescrever Pedra em Isabelle/C.
- ∀ interleavings de `ConcurrentDb` (PCT/TSan).
- L28 TCP como teorema; liveness sem axiomas ES.
- io_uring ring no modo verificado; prova de mídia `fsync`.
- Benches / crates.io / RFC-0153–0159.
