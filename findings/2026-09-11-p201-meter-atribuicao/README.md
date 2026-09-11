# P201 meter — atribuição kvrocks_set_mc50 na árvore viva (pós-wipe)

**Data:** 2026-09-11 06:04Z | **Caixa:** caixote `linux-gate-p149b` (CHV, 4 vCPU,
Ryzen Threadripper PRO 3975WX, guest Alpine 6.18.35, nproc=4)
**Imagem:** `ghcr.io/paulocsanz/pedradb-linux-gate:p201n`
(digest `sha256:aab281a0c92d…58f27fe`, base p04a, src = árvore viva
2026-09-11T01:24-03:00, head 231addb3 + commit ff233b15 do kernel 0201)
**Protocolo:** mesmo boot, 3 rounds quiet (load1<2 ×2), ordem rotativa por
round, peer RocksDB default `ROCKS_PARITY_SYNC=0`, Pedra
`PEDRA_PARITY_ASYNC=1` (coluna same-class), `ROCKS_YCSB_OPS=2000` (×50
clientes = 100k ops/run), `ROCKS_PARITY_RATIO_FLOOR=none`, G1 unset.

## Resultado (ratio = pedra_qps / rocks_qps do MESMO round)

| variante | r1 | r2 | r3 | min | mediana |
|---|---|---|---|---|---|
| default (bypass, unfair, spin 0) | 0,9936 | 0,9605 | 1,1762 | **0,9605** | 0,9936 |
| fair (`PEDRA_WRITE_FAIR=1`) | 0,3319 | 0,3582 | 0,3843 | **0,3319** | 0,3582 |
| group (`PEDRA_ASYNC_GROUP=1`) | 2,1029 | 1,5191 | 2,5656 | **1,5191** | 2,1029 |

QPS crus: rocks 83,6k/91,5k/139,8k (r3/r1/r2); group 192,5k/212,4k/214,5k;
default 91,0k/134,3k/98,3k; fair 30,4k/50,1k/32,1k.

## Leitura

1. O 0,37× nomeado do cartaz NÃO reproduz no default da árvore viva
   (paridade). A perda 0,37× É a política de fair handoff
   (`PEDRA_WRITE_FAIR=1`): 0,33–0,38× — liberar o lock acordando a fila
   inteira um-a-um serializa o herd de 50 writers.
2. O merge assíncrono por grupo (líder encoda por todos, sem catch-up wait)
   dá **1,52× min-of-3, 2,10× mediana** vs Rocks default em mc50 — o maior
   ganho medido nesta célula. Hoje default OFF (decisão RFC-0044-p1).
3. Corte 0201 re-alvejado: group default ciente do eixo cliente. REGRESSÃO
   PENDENTE: medir group vs default nas outras formas (escritores 1/4,
   apply_mc4, ycsb writes) antes de virar default — o 0044 desligou por
   A/B de 5 rounds (ver concurrent.rs ~L178 e findings/rfc0044-p1).

Serial bruto: `caixote logs linux-gate-p149b` (boot 06:02:43Z BUILD_OK;
P201_CELL/P201_METER_RESULT acima). Entry: scratch `p201_entrypoint.sh`.
