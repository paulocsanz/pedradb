# Host gate bloqueado — meter Linux indisponível (2026-09-10 18:32 −03)

**Gate:** guest `linux-gate-p149b` resolúvel **∧** Darwin load1 < 8.
**Resultado desta checagem (fresca, na hora da adjudicação):**

- `ping linux-gate-p149b` → `cannot resolve ... Unknown host`
- `ssh linux-gate-p149b` → `Could not resolve hostname`
- Darwin `uptime` → `load averages: 36,52 20,24 14,88`

Ambas as pernas falham. Precedente: 0190/0193 (findings
`2026-09-10-rfc0193-p05-meter-blocked.md`). **Nenhum número Linux é
fabricado**; as fatias de meter fecham `blocked` com data, Darwin = DIAG.

**Re-check da onda 0195 (19:33 −03):** `ping` → `Unknown host`;
`uptime` → `load averages: 37,93 23,01 19,71`. Gate segue fechado —
as fatias de meter do 0195 (P0.4/P1.x/P2.2/P2.3) fecham `blocked` por
este mesmo finding.

**Re-check da onda 0197 (22:24 −03, com tentativa de recuperação):**
restart do OrbStack feito (`quit` + `open -a OrbStack`; daemon sobe) e a
VM continua sem rede — `vmgr.log` repete `host-unix forward: dial failed
addr={1 0.250.250.2 2375} connection refused`; `ssh linux-gate-p149b` →
`Could not resolve hostname`; `orb list` pendurado (12 s bounded, morto);
Darwin `uptime` → `load averages: 36,22 28,44 17,73`. Sem reset de
fábrica (host compartilhado e ocupado — destrutivo demais). Gate segue
**fechado**: as fatias de meter do 0197 (P1.1–P1.4) fecham `blocked`
por este mesmo finding.

## Adendo 2026-09-11 00:45 −03 — diagnóstico fundo a pedido do usuário

Pergunta do usuário: "quase certeza que o host tá ok, você que tá usando
errado". Investigação completa (captura em
`{SCRATCH}/host-gate-investigation.txt` desta sessão):

**O host (este Mac/Hackintosh) está saudável — a perna load1 deixou de
ser o bloqueio:** disco 107 GiB livres; RAM 96 GiB com 79% livre; swap
241 MiB/1 GiB; load1 chegou a **7,88** (abaixo do corte de 8) durante a
re-chechagem. Leitura corrigida: o gate não fecha mais por load.

**O bloqueio real é a userland da VM do OrbStack, reproduzido em 2
restarts limpos** (`quit` + `pkill Helper` + `open -a OrbStack`, esperas
de 20 s e 90 s):

- `vmgr.log` de boot: **todas** as fases completam (`create_vm`,
  `start_vm`, net services) — o hypervisor sobe a VM.
- Docker engine na VM: `connection refused` em `0.250.250.2:2375`
  (pilha de rede viva, listener ausente) — erro em loop no `vmgr.log`.
- SSH da VM (ponte oficial `Host orb`, ProxyCommand `ssh-proxy-fdpass`):
  `Connection timed out during banner exchange` — sshd não responde.
- Todos os caminhos oficiais testados e mortos: `orb -m`, `orb list`,
  `orbctl list` (pendurados), `docker ps/version` (EOF/refused),
  `ssh orb`, sondas diretas 192.168.194.2-4:22 (nada).
- `linux-gate-p149b` não é máquina OrbStack (`~/.orbstack/config` só tem
  `docker.json`) nem VM Lima/Colima/QEMU (`~/.lima` vazio, sem qemu) —
  é container/overlay **dentro** da VM do OrbStack, e o CPU reportado
  (Threadripper PRO 3975WX) é o CPU físico repassado pela VM.
- Conclusão de uso: não é erro de invocação — todas as pontes oficiais
  falham idênticas com a VM booted-muda. Ações que só o usuário pode
  fazer: (1) abrir o GUI do OrbStack e olhar o erro da VM/Linux;
  (2) atualizar o OrbStack (2.2.3); (3) último recurso, factory reset —
  **destrói a imagem do guest** (o backing do p149b morre junto).

O meter 0198 P1.1 (mc50 antes/depois) continua `blocked` por este gate.

## Adendo 2026-09-11 01:30 −03 — o adendo 00:45 está ERRADO; gate ABERTO

O usuário corrigiu: "o linux é do caixote". O adendo 00:45 acima errou o
alvo: **`linux-gate-p149b` não tem nenhuma relação com o OrbStack local**.
É um serviço da plataforma caixote (projeto `pedradb-dst`, env production,
`cloud-hypervisor`, 4 vCPU/4 GiB, IP 10.0.0.113, container
`cnt_4a95c576…`), rodando num host físico próprio (o Threadripper,
`ssh paulo@192.168.68.109`, 64 CPUs, Ubuntu 24.04) que também roda o
`caixote-api`. O CPU Threadripper reportado pelo guest é repassado pelo
CHV da plataforma — a coincidência que enganou o 00:45.

Estado real das pernas do gate (2026-09-11):

- Guest: **vivo e legível** — serial via `caixote logs linux-gate-p149b`
  (última atividade pré-meter: WARM10-CELL 2026-09-10T01:41Z).
- Darwin load1: 7,88 < 8 — passa.
- O caminho do meter NÃO é exec/ssh (o guest não tem sshd;
  `service exec` dá 501 no cloud-hypervisor): é **bake de imagem +
  `caixote service deploy-image` + serial**. `caixote push` não
  (source-builder partido, RFC 0192/p11). Local: `crane append` sobre
  `ghcr.io/paulocsanz/pedradb-linux-gate:p04a` com as fontes + entrypoint
  (auth ghcr do `~/.docker/config.json`).

**Gate re-adjudicado: ABERTO.** O meter P1.1 do RFC-0201 (renumerado;
colisão com a 0198 composição-registrada) está rodando: imagem `p201m`
deployed com matrix same-boot default/fair/group × 3 rounds
(`kvrocks_set_mc50`, peer `sync=false`). As linhas "blocked" das tabelas
abaixo ficam como registro histórico do dia 2026-09-10 e serão
re-adjudicadas pelos meters que rodarem.

Registros correlatos deste dia: o wiring aterrado em 2026-09-10 23:42
(testes 7/7) foi destruído às 23:49:31 por um `git reset --hard` da
sessão paralela (composição-registrada) — ver RFC-0201 adendo.

## Fatias bloqueadas por este gate (adjudicadas hoje)

| RFC | Fatia | veredito |
|---|---|---|
| **0195** | **P0.4 meter prefix 100M @4GiB (com e sem a condição) + regressão point-get quente** | **blocked (re-check 19:33: guest `Unknown host`, load1 37,93)** |
| **0195** | **P1.1 re-split quieto / P1.2 meter 0193 P0.5 / P1.3 meter 0194 P0.4 / P1.4 ycsb_b** | **blocked (mesma re-check)** |
| **0195** | **P2.2 U-cells / P2.3 Grid B** | **blocked (mesma re-check)** |
| **0195** | **P2.1 ycsb_f rmw restante** | **todo-condicional: atrás do meter P1.2 — gate fechado, não medível** |
| 0194 | P0.4 meter 15M/25M @4GiB leftover overwrite | blocked (DIAG-only) |
| 0194 | P1.1 meter 0193 P0.5 (overwrite 10k / mc50 / apply_mc4) | blocked |
| 0194 | P1.2 re-split quieto pós-0193 + re-pin fixture 0192 | blocked (fixture fica labeled-stale) |
| 0194 | P1.3 prefix 100M Linux 3-run | blocked |
| 0194 | P1.4 ycsb_b Linux 3-run | blocked |
| 0194 | P2.3 U-cells lote 3-run | blocked |
| 0194 | P2.1 publish unification (condicional no P1.2) | non-condition: o disparador (re-split nomeando publish > ~0,4 µs/op em fills>0) não pode ser medido — não dispara |
| **0197** | **P1.1 meter 100M write / P1.2 15M+25M same-box re-fit / P1.3 prefix 100M + re-split / P1.4 eixo clientes** | **blocked (re-check 22:24: guest unresolvable pós-restart, load1 36,22)** |
| 0192 | P1.1 perna Linux quieta pós-0190 (`name_cut` pós-guarda) | blocked |
| 0192 | P1.2 erro do modelo vs medido na mesma perna | blocked (a aritmética `qps_hat_error_permille` ATERIZOU no kernel — só a perna falta) |

## O que isso NÃO bloqueia (aterrissado nesta onda, sem Linux)

- 0194 P0.1/P0.2/P0.3 (kernel + wiring off-lock + telemetria opt-in) — testes nomeados verdes, A/B serial zero falhas novas, musl exit 0.
- 0192 P2.1 (lane_hist no JSON do compare) — aterrissado (`Engine::lane_histogram` → block JSON).
- 0192 kernel: `qps_hat_error_permille` (3 testes) — a perna de medição aplica o número quando o gate abrir.
- 0197 P2.1 (âncoras GET datadas no kernel + CLI) — aterrissado com o gate fechado:
  escada 100M em 2 caixas registrada (`GET_SIDE_ANCHORS_2026_09_10`, 3 testes
  `rfc0197_p21_*` + CLI); 10k/2M/15M/25M GET seguem deflexão nomeada (sem
  medição datada), re-ancoradas pelos meters P1.2/P1.3 quando o gate abrir.

Re-abrir: qualquer onda futura refaz esta checagem; gate passando ⇒ meter
3-run quieto STOP/CONT warm10, peer `ROCKS_PARITY_SYNC=0`, min-of-3.
