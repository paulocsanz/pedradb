# RFC-0211 P0.3 — meter rmw-mc4 no gate: 4 braços × 3 rounds, veredito datado (fronteira validada como mecanismo; alvo ≥1,0 não fechado no min — opt-in mantido)

**Data:** 2026-09-11 (p211m 2026-09-12T00:17–00:26Z) | **Caixa:** caixote
`linux-gate-p149b` (CHV, 4 vCPU, guest Alpine, nproc=4 — regime-alvo
`writers == ncpu` por construção) | **Imagem:** `p211m` digest
`sha256:eb374a05…` (base `p04a`; src = árvore com P0.1+P0.2 do RFC-0211
commitado `4c93d2c5`) | **Peer:** RocksDB default
`ROCKS_PARITY_SYNC=0`; pedra `PEDRA_PARITY_ASYNC=1` (coluna same-class).

**Protocolo (desenho do RFC, executado idem):** mesmo boot, 3 rounds
quiet (gate load1<2 ×2 20s; quiet em 1,12 antes do r1), round-major com
braços {clean (âncora), `PEDRA_RMW_SCHED=1` (rmw), RMW_SCHED+
`PEDRA_WAL_BUFFER=1` (rmwbuf), `PEDRA_WAL_BUFFER=1` (buf)}; por
(round, braço): 2 engines × 3 invocações — MC
(`ycsb,deps`: ycsb_a_mc4, ycsb_f_mc4, deps_cache_overwrite_mc4,
deps_apply_batch_mc4), SINGLE (`ycsb_f`), KVR (`kvrocks_set`,
`kvrocks_set_mc50`); `ROCKS_PARITY_CLIENTS=4`, `ROCKS_YCSB_OPS=2000`,
`ROCKS_YCSB_DIST` unset, `timeout 900`; 72/72 invocações rc=0 (zero
`M-FAIL`, zero `MISSING`); `RESULT=P211M_DONE 00:25:56Z`. Captura bruta:
janela completa do console no scratch (`p211m-raw.json`,
`p211m-summary.txt`). Nota de carga: load1 sobe 1,12→4,55 ao longo dos
rounds (inerente ao back-to-back); cada ratio é pedra/rocks do MESMO
braço medidos a ~2s de distância — o par via o mesmo load.

**Âncoras re-ancoradas same-class (não comparáveis cross-boot por
régua; classe reproduzida):** clean 0,491 min (piso p209b 0,532); buf
med 0,786 (p209b 0,780).

## Resultado — ratio vs rocks do MESMO braço, min/med (r1/r2/r3)

| célula | papel | clean | rmw | rmwbuf | buf |
|---|---|---|---|---|---|
| **ycsb_f_mc4** | **alvo** | 0,491/0,622 [0,491/0,622/0,639] | **0,836**/1,352 [1,390/1,352/0,836] | 0,552/1,039 [1,040/1,039/0,552] | 0,591/0,786 [0,591/0,786/1,097] |
| ycsb_a_mc4 | guarda | 0,871/0,920 [1,266/0,871/0,920] | 0,896/1,309 [1,309/2,194/0,896] | 0,703/0,774 [0,774/0,978/0,703] | 0,604/1,148 [1,399/0,604/1,148] |
| deps_cache_overwrite_mc4 | guarda | 0,953/1,038 [0,953/1,368/1,038] | 1,357/1,509 [1,357/1,984/1,509] | 0,913/1,313 [1,511/1,313/0,913] | 1,236/1,512 [1,797/1,236/1,512] |
| deps_apply_batch_mc4 | guarda (multi-op) | 1,285/1,397 [1,285/1,397/1,675] | 1,351/1,355 [1,358/1,351/1,355] | 1,258/1,262 [1,258/1,466/1,262] | 1,173/1,257 [1,173/1,257/1,371] |
| ycsb_f (1c) | single | 1,097/1,920 [1,920/1,097/2,073] | 1,038/2,029 [2,029/1,038/2,077] | 1,655/2,002 [2,002/1,655/2,225] | 2,044/2,109 [2,109/2,278/2,044] |
| kvrocks_set (1c) | single | 2,285/2,500 [2,285/2,500/2,522] | 2,319/2,426 [2,426/2,319/5,146] | 2,308/2,456 [2,308/2,456/2,668] | 2,378/2,440 [2,458/2,378/2,440] |
| kvrocks_set_mc50 | guarda (paga 1,678) | 2,343/2,635 [2,343/2,635/2,835] | 2,749/2,857 [2,749/2,857/3,744] | 2,438/2,494 [2,494/2,438/2,548] | 2,345/2,442 [2,442/2,345/3,100] |

**Guardiãs clean→rmw (min):** ycsb_a +2,9%; deps_cache_overwrite +42,5%
(escrita 1-op também entra no regime novo — melhora, não regrediu);
deps_apply_batch +5,1% (multi-op fica no bypass por construção, como
desenhado); kvrocks_set_mc50 +17,3%. Singles: kvrocks_set +1,5%;
ycsb_f 1c min −5,4% com med +5,7% — caminho byte-idêntico por
construção (writers=1 nunca entra no regime novo; guarda do kernel
`writers ≤ 1 ⇒ Bypass`), tratado como ruído de round único
(1,038 vs 1,097), registrado honestamente.

## Telemetria por braço (oráculo de merge)

O bench só carrega contadores `write_group` em `kvrocks_set_mc*` e
`deps_apply_batch mc*` (lib.rs 1098/2467) — `kvrocks_set_mc50` reportou
`groups=103024 avg_group=1,00` idêntico nos 4 braços (o env não toca o
caminho daquele harness, como esperado; o cartaz pago 1,678 fica
intocado). O oráculo de merge da célula-alvo é o teste nomeado in-repo
`rfc0211_env_axis_rmw_sched_forms_groups` (P0.2: com env, grupos
formados de verdade, `queued > 0`, avg ≥ 2; sem env, `queued == 0`).
**Limitação registrada:** os `run.log` por-shape e o
`write_phase_stats` (lock_wait vs wal vs publish) ficaram no `/tmp` da
VM (sem exec/ssh no caixote) — não capturáveis pós-run; a fatia P1.2
estende o entrypoint para emiti-los ao console.

## Veredito (2026-09-12T00:25:56Z, rfc0211 P0.3)

- **O mecanismo funciona e é seguro:** `PEDRA_RMW_SCHED=1` sobe o piso
  do alvo de 0,491 → **0,836 min** (+70%; med 0,622 → 1,352, +117%),
  sem regredir nenhuma guardiã >5% (todas subiram no min) — a fronteira
  0201 estendida ao grupo async 1-op drena o grupo em vez de serializar
  por writer.
- **Perda honesta vs o gate ≥1,0:** min-of-3 = 0,836 < 1,0 (faltam ~20%).
  Rounds 1–2 do braço rmw: 1,390/1,352 em load1 1,35–2,98; o round 3
  (load1 4,55) cai a 0,836 — o residual parece acoplado a load/get-side
  (alternância read/write-lock do `get` do rmw), não à syscall WAL
  (rmwbuf não fecha: min 0,552, dispersão alta).
- **Decisão P1.1 (regra do RFC): min <1,0 ⇒ sem flip de default —
  opt-in mantido.** Próxima fatia nomeada: **P1.2** decomposição do
  residual (`write_phase_stats` por braço no console) + **P2.1** sweep
  do eixo writers (mc2/mc6/mc8) — nunca silêncio.
- Cartazes pagos NÃO re-medidos: apply_mc4 1,0859 e kvrocks_set_mc50
  1,678 aparecem só como guardiãs in-arm; os números pagos ficam.
