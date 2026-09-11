# P201r meter — células mc4 do eixo cliente (apply/overwrite/ycsb_f)

**Data:** 2026-09-11 07:51:54Z | **Caixa:** caixote `linux-gate-p149b`
(CHV, 4 vCPU, guest Alpine, nproc=4)
**Imagem:** `ghcr.io/paulocsanz/pedradb-linux-gate:p201r2`
(digest `sha256:0ec55b38…`, src = árvore viva com corte P0.3 `f7b2c20f` +
clamp P0.1 `b2b0295b` + kernel 0192 `b959428a`)
**Protocolo:** mesmo boot, 3 rounds quiet (load1<2 ×2), ordem de suíte
rotativa, `ROCKS_PARITY_CLIENTS=4` nos DOIS engines (suítes ycsb+deps
completas — seeding idêntico ao p201o), Pedra default env-limpo
(`PEDRA_PARITY_ASYNC=1`, coluna same-class), peer RocksDB default
`ROCKS_PARITY_SYNC=0` do MESMO round, `ROCKS_YCSB_OPS=2000`,
`ROCKS_PARITY_RATIO_FLOOR=none`.

**Lição v1 (07:28Z, imagem `p201r` descartada):** os harnesses mc de
ycsb/deps são gated por `ROCKS_PARITY_CLIENTS` (o sufixo `_mcN` é o valor
do env), NÃO por `ROCKS_PARITY_ONLY`; sem CLIENTS o bench roda vazio com
rc=0 e JSON sem benches. `kvrocks_set_mc50` é caso especial (50 hardcoded
em `run_kvrocks`). A v1 produziu INCOMPLETE rounds=[] — nada medido.

## Resultado (ratio = pedra_qps / rocks_qps do MESMO round)

| forma | r1 | r2 | r3 | min | mediana |
|---|---|---|---|---|---|
| deps_apply_batch_mc4 | 1,0859 | 1,1160 | 1,3308 | **1,0859** | 1,1160 |
| deps_cache_overwrite_mc4 | 0,3698 | 0,4611 | 0,4722 | **0,3698** | 0,4611 |
| ycsb_f_mc4 | 0,2947 | 0,6338 | 0,3195 | **0,2947** | 0,3195 |

QPS crus: apply pedra 7,7k–9,3k vs rocks 6,5k–8,6k; overwrite pedra
82k–129k vs rocks 173k–349k; ycsb_f pedra 146k–189k vs rocks 247k–640k
(o rocks oscila 2,6× entre rounds — min-of-3 absorve).

## Adjudicação

1. **`deps_apply_batch_mc4` PAGO (cartaz):** a célula 0,47× (Linux 0183)
   agora mede **1,086× min-of-3 quiet** vs Rocks `sync=false` — o
   off-lock ticket (0193) + o regime de merge por eixo (0201 P0.3)
   entregaram. Célula acima do floor 1.0 (RFC-0041).
2. **`deps_cache_overwrite_mc4` = 0,370× min: buraco real, reproduzível**
   (0,37/0,46/0,47). 4 writers 1-op (== ncpu ⇒ bypass por política 0201:
   lock de escrita próprio + `commit_async_one` cada). O Rocks a 173–349k
   qps é WAL bufferizado (memcpy por op, sem syscall); a Pedra paga
   `write()` por op no bypass — mesma classe do buraco single-client do
   sweep (kvrocks_set 0,67–0,75×, cache_overwrite 0,51–0,54×; ver
   `findings/2026-09-11-p201o-sweep/`), agravada pela contenção do lock
   de escrita sob 4 clientes.
3. **`ycsb_f_mc4` = 0,295× min: buraco real** (rmw = get + write 1-op por
   op) — mesma classe do anterior com o get somando.

**Dono nomeado (hipótese hat, precisa meter de atribuição):** o caminho
async 1-op (`commit_async_one`/bypass) faz uma syscall `write()` por op;
ataque candidato = buffer de WAL em user-space com `write()` agrupado
(coalescing por limite de bytes/idle) SEM esperar por writers (nenhum
wait-to-grow — 0180/0190 vetados: o flush é por tamanho/timeout do
buffer, não por espera de grupo). Esse é o P0 candidato do próximo RFC de
escala; as células medidas aqui são as âncoras.

Serial bruto: `caixote logs linux-gate-p149b` (P201R_CELL /
P201R_METER_RESULT acima; captura `p201r-full.log` v1 +
monitor v2). Entry: scratch `p201r2_entrypoint.sh`.
