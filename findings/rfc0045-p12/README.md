# RFC-0045 P1.2 — bissecção do flip do `deps_lock_prewrite` (0.94 in-suite vs 2.2× "isolado")

2026-08-21, 19:01–19:12. Carga registrada por run (10–32; suja — números de
mecanismo/contraste, não coluna oficial). 9 rounds pareados no total:
`PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`, suíte completa ycsb+deps em um
processo.

## Premissa do P0.3 corrigida primeiro

Os runs "isolados" do P0.3 (`findings/rfc0045-p0/lockprewrite-r{1,2,3}/`)
**não eram isolados**: os JSONs contêm as 14 shapes (ycsb_a..f + deps
todas, n=200 cada). O 2.28/2.26/2.09 veio de suíte completa em
**1024/200/100/uniform**; o 0.94/1.40/0.77 do árbitro quieto (04c7aa2,
load 9.4) veio de suíte completa em **4096/2000/1000/zipfian**. O contraste
real era **config**, não contexto de suíte. ("Prefixos crescentes do v0"
perdeu o objeto: os dois lados já rodavam o mesmo contexto.)

## Bissecção 1 — eixos de config (HEAD, load 11–14)

Um eixo por vez a partir do small; `baseline-big` = todos os eixos juntos.

| config | pedra | rocks | ratio |
|---|---:|---:|---:|
| small 1024/200/100/uniform | 48 959 | 24 312 | **2.01** |
| +records 4096 | 43 839 | 20 232 | 2.17 |
| +ops 2000 | 43 902 | 19 447 | 2.26 |
| +payload 1000 | 29 204 | 16 321 | 1.79 |
| +zipfian | 47 343 | 23 042 | 2.05 |
| big 4096/2000/1000/zipfian | 25 423 | 14 918 | **1.70** |

Nenhum eixo flipa; o config inteiro também não.

## Bissecção 2 — HEAD vs commit do árbitro, mesmas condições

| lado | rounds | ratios | p99 pedra | max pedra |
|---|---|---|---:|---:|
| HEAD, load 10–15 | big r1–r3 | 1.66 / 1.64 / 2.77 | 0.1 ms | — |
| **04c7aa2** (commit do 0.94), load 16–32 | big r1–r3 | 1.92 / 1.80 / 2.38 | 0.14–0.22 ms | 2.3–10.2 ms |

O flip não reproduz nem no próprio commit que mediu 0.94. Não é
código-estado.

## O perfil do caso flipado (dos JSONs originais do árbitro)

| run | pedra qps | p50 | p99 | max | rocks qps | p50 | max | ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| q-r1 | 15 494 | 0.035 | 0.09 | 15.5 | 11 049 | 0.059 | 12.8 | 1.40 |
| q-r2 | 7 736 | 0.038 | 0.58 | **70.9** | 8 260 | 0.059 | 22.5 | 0.94 |
| q-r3 | 7 169 | 0.034 | 0.06 | **131.5** | 9 317 | 0.059 | 19.9 | 0.77 |

**O flip é 100% cauda**: p50 do Pedra é 1.6–1.7× melhor que Rocks em todos
os runs, inclusive nos flipados. O que crushou o qps foram stalls de
20–130 ms exclusivos do Pedra dentro da shape (p99 0.58 ms no r2; max
70/131 ms). Stalls que não reaparecem em nenhum dos 9 rounds desta bissecção
(nem no HEAD nem no 04c7aa2, carga 10–32).

## Verdict P1.2

1. **Refutado**: flip por contexto de suíte (premissa original era
   incorreta — os "isolados" eram suíte completa), por eixo de config, e por
   commit (controle no próprio 04c7aa2).
2. **O que o 0.94 era**: dois rounds de uma janela de 2 min (16:53–16:55 de
   08-20) com stalls raros de 20–130 ms no lado Pedra. Causa mecânica dos
   stalls **não determinada** — não reproduz em 9 rounds desde então.
   Candidato estrutural (não provado): fold de parked imms pelo compact
   worker (fires com 1 writer + ≥2 parked; precisa ~512 MB escritos — só o
   big config chega, e nem sempre). Se voltar: instrumentar park/fold do
   worker, o `max_ms` da shape é o tell.
3. **Número em pé do shape**: ≥1.6× no big config (mediana 1.66–1.70, 6
   rounds sujos + 3 controles), 2.0–2.3× no small. O árbitro oficial
   continua sendo P1.3 (quieto 3×, load <10).
4. Nota: a gate do rerun "quieto" teve bug de locale (vírgula decimal) — os
   rounds HEAD rodaram a load 10–15, não <10. Não muda o verdict (nenhum
   round flipou em nenhuma carga testada), mas P1.3 segue necessário para a
   coluna oficial.

## Raw

- `axis sweep` JSONs: `findings/rfc0045-p12/{baseline-small,axis-*,baseline-big}/`
- HEAD borderline rounds: `findings/rfc0045-p12/quiet/{big-r1..3,small-c}/` + `loads.txt`
- 04c7aa2 control: `findings/rfc0045-p12/old-04c7aa2/{r1..3}/` (worktree `/tmp/pedra-04c7aa2`)
- Logs: `p12-sweep.log`, `quiet.log`
