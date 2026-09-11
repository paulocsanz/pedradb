# RFC-0210 P1.2 — veredito datado dos 7 fantasmas de catálogo l28

Data: 2026-09-11
Par: P1.2 do `docs/rfc/0210-drenar-l28-protocolo-tcp-real-fim-fantasma.md`
Fantasmas: `l28_tcp_{add, cnew, svget, newget, jleft, caught, grown}` (7)

## Medição (HEAD de 2026-09-11, antes da cirurgia)

1. **Kernels.** `grep -n "pub fn l28_tcp_" crates/pedradb-store/src/l28.rs` — os
   kernels TCP terminam em `l28_tcp_pj_ok` / `l28_tcp_pj_ok_as_is` (linhas
   399–405). Nenhum `fn l28_tcp_{add,cnew,svget,newget,jleft,caught,grown}_ok`
   existe em `crates/` (grep direto vazio).
2. **Handlers nomeados como `live_callers` em `cluster_real.rs`.**
   `client_add_member_joint` não é importado/usado (lista de imports linha 57:
   `client_get, client_leave_joint, client_put, client_remove_member_joint,
   client_status`); `tcp_node_disk_added_joint` /
   `tcp_node_disk_caught_up` não existem em lugar nenhum de `crates/`
   (grep `fn tcp_node_disk_added_joint|fn tcp_node_disk_caught_up` vazio).
   `client_get` / `client_status` / `tcp_node_disk_high_water` EXISTEM como
   utilitários, mas `tcp_node_disk_high_water` já é a planta do atom pago
   `l28_tcp_hw` (0126 P1.2) — fantasma `grown` era duplicata sem corpo.
3. **Planta.** `l28_real_tcp_add_member_joint_cnew` não existe em
   `crates/pedradb-store/tests/l28_real_tcp.rs` (31 testes no arquivo, nenhum
   com esse nome; a lista termina em `l28_real_tcp_plant_joint`).
4. **Binário real.** `grep 'add_member\|AddMemberJoint'
   crates/pedradb-store/src/bin/cluster_real.rs` — VAZIO. O caminho add-member
   de produção NÃO existe no protocolo TCP real: o helper wire tag 20
   (`client_add_member_joint`) em `tcp.rs` não é despachado pelo binário. O
   caminho in-process (`Store::add_member_joint`, RFC-0119/0120) já está
   coberto pelos pares reais do 0208.
5. **Lean.** Zero defs `l28_tcp_{...}_ok` em `formal/aeneas/lean/*.lean`
   (grep vazio). Origem: os nomes apareceram em `36d4f685` só em catálogo /
   `verified.rs` / `docs/status.md` (já documentado em
   `formal/aeneas/EXTRACT.md`, seção "Catalog `entry`s with no Lean `def`").

## Veredito por fantasma

| fantasma | veredito | recusa (por que não re-escrever) |
|---|---|---|
| `l28_tcp_add` | APOSENTADO | sem `fn`, sem handler no binário real, sem planta |
| `l28_tcp_cnew` | APOSENTADO | C-new em eleição já é o atom pago `l28_tcp_peer` (0140) |
| `l28_tcp_svget` | APOSENTADO | sem `fn`, sem planta; sv-get não existe no protocolo |
| `l28_tcp_newget` | APOSENTADO | sem `fn`, sem planta; new-get não existe no protocolo |
| `l28_tcp_jleft` | APOSENTADO | `tcp_node_disk_added_joint` não existe; joint-leave in-process é do 0208 |
| `l28_tcp_caught` | APOSENTADO | `tcp_node_disk_caught_up` não existe; catch-up é snapshot-carried, sem kernel |
| `l28_tcp_grown` | APOSENTADO | duplicata sem corpo do atom pago `l28_tcp_hw` (0126) |

Nenhum caminho vivo para re-escrita: o produto decide construir, não o
catálogo. Nunca inventar gate de identidade para agradar o catálogo.

## Cirurgia (um commit)

- `scripts/formal/catalog.json`: −7 pares (299→292; campaign 33→26).
- `scripts/formal/residuals.json`: `glue.data_fate` 68→61;
  `glue.single_artifact` 291→285 (corrigiu também contagem stale 291 vs live
  292 — o marker do ledger dizia 292); nota datada de aposentadoria no
  `close` da residual `R-joint` (linha continua `continuous`).
- `scripts/ratchet/proof_depth.tsv`: `cap_data_fate` 68→61 (escaldão mais
  apertado; data_fate=61<=61 verde).
- `docs/verification-ledger.md`: marker `total=292 proof=266 campaign=26
  absent=0 single_artifact=285 aeneas_scripts=224 clones=7 models=34`.
- `formal/aeneas/EXTRACT.md`: veredito datado "retired" na seção dos
  fantasmas + plano do 0208 atualizado.
- `docs/rfc/0210-...md`: bloco de veredito + status `done` + tabela.

## Verificação (verde ANTES e DEPOIS da cirurgia)

- `scripts/check_depth_floor.py` — GREEN antes (data_fate=68<=68) e depois
  (data_fate=61<=61; extract 215, atom 63, close 6, count 7 inalterados —
  fantasmas nunca tiveram linha TSV).
- `scripts/check_product_floor.py` — GREEN antes e depois.
- `scripts/check_ledger_consistency.py` — GREEN antes (total=299 proof=266
  campaign=33) e depois (total=292 proof=266 campaign=26; 19 ponteiros
  resolvem — nenhum ponteiro do ledger nomeava fantasma).
- `crates/pedradb-core` `cargo test --test host_anchor_table` (âncoras
  RFC-0203/0204) — 3 passed ANTES (5.77s) e DEPOIS.
- `scripts/formal/test_proof_vs_campaign.py` — ok (live proof vs campaign;
  nenhuma linha fantasma em `close_proofs.tsv`/`proof_depth.tsv`, grep vazio).
- `scripts/formal/test_twin_mutation.py` — ok.

Nenhuma conta ancorada quebrou: `total` do ledger é derivado de `len(pairs)`
(não pinado em 299), `host_anchors.tsv` é tabela de timing de fdatasync
(0203/0204) e não conta pares, e nenhum script pinava 299.
