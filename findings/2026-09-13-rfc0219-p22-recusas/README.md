# RFC-0219 P2.2 — recusas medidas por sítio (2026-09-13)

Fila `concurrent.rs` medida no fim do P2.2 (grep datado abaixo):
**6 linhas** = 3 sítios de código recusados + 3 linhas de
comentário/doc (nunca foram código). O P2.2 fecha com: 19 sítios de
código resolvidos — **2 pares nascidos átomo** (pull 19
`flusher_gate_plan` ×5 portões; pull 20 `parked_debt_plan` ×2 portões),
**9 drenos** a `match` em kernels já pareados (commit f0671326) e
**3 recusas nomeadas** — o número publicado que fecha o alvo datado
do P2 (RFC-0219 «Meta mensurável»).

## Recusa por grupo (o porquê medido de cada um)

**R4 — `if refuse_publish` (1 sítio: 1452, `finish_group_off_lock`).**
A decisão de fate JÁ vive no match pareado 5 linhas acima
(`wal_commit_plan(need_sync, failed)` → `AppendSyncFence`) e no
kernel pareado `may_publish_group(!failed)` que semeia o braço
complementar; o `if` apenas despacha no local derivado. Um plano enum
para o produto (wal_commit_plan × may_publish_group) re-implementaria
o produto dos dois kernels — spray de cola, não kernel novo.

**R5 — `if batch_is_empty(g.unsynced_sst_count() as u64)` (1 sítio:
3402, `persist_unsynced_l0s_off_lock`).** Guarda de ausência-de-
trabalho, não decisão de fate: nada acumulado → nada a persistir. O
kernel `batch_is_empty` já é átomo pareado (RFC-0171 P1.1,
`batch_is_empty_ok_iff_zero`) e continua VIVO no sítio; embrulhá-lo
num plano é o anti-padrão wrap-`is_empty` nomeado pelo RFC (nunca
wrap `is_empty` para virar número). O data-fate da função (publicar
MANIFEST) é decidido no dreno `manifest_publish_plan` (3379).

**R1 — `if let Err(e) = Db::fsync_sst_paths(...)` (1 sítio: 3412,
`persist_unsynced_l0s_off_lock`).** A mesma família R1 do P2.1 ×9: o
corpo propaga `e` (`restore_unsynced_ssts(paths)` + `return
Err(e)`); a "decisão" é o desfecho da própria I/O — território Env,
cânone: Env não é provável.

## Linhas de comentário (3 — nunca código)

7 (doc do módulo), 2703 (doc fence acima de `recover_from_fence`),
6999 (doc fence case B) — casam no grep por conterem as palavras, não
são portões.

## Alvo datado fechado (número publicado)

- Re-datado no P2.1: fim = (288 + P₂.₂)/(310 + P₂.₂), teto sem recusa
  310/332 = 93,37% (P₂.₂ ≤ 22 − R_c).
- Medido: P₂.₂ = 2 pares (5+2 portões pagos por 2 planos — a fila não
  é 1 sítio = 1 par), 9 drenos sem par, R_c = 3.
- **Fim publicado: 290/312 = 92,95%** (início do objetivo 270/292 =
  92,47% @80782f6c). O teto 93,37% não foi alcançado porque 19 sítios
  de código ≠ 19 pares: 7 portões pagos por 2 planos, 9 já tinham
  kernel pareado (dreno), 3 são Env/anti-padrão medidos acima.

## Captura do grep (2026-09-13, pós-drenos, HEAD f0671326)

```
$ grep -n -E 'if .*(sync|flush|durable|visible|publish|fence|fsync)' crates/pedradb-core/src/concurrent.rs
7://!   lock for **one** `fsync` (if any member requested sync), then reacquires
1452:        if refuse_publish {
2703:    /// class, uncertain range), if this Db was ever fenced.
3402:            if crate::write_admission_kernel::batch_is_empty(g.unsynced_sst_count() as u64) {
3412:        if let Err(e) = Db::fsync_sst_paths(&env, &dir, &paths, sync) {
6999:    /// sees it if the frame reached the file (existing fence case B).
```

6 linhas: 3 doc + as 3 recusas nomeadas (R4 1452, R5 3402, R1 3412).
