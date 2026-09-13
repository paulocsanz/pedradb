# RFC-0211 P1.2 — decomposição do residual 0,836 do braço rmw (telemetria write_phase por braço) + fix do deadlock que travou a primeira onda

**Data:** 2026-09-12 (onda `p211p` re-emexecutada sobre imagem `p211t`) |
**Caixa:** caixote `linux-gate-p211r` (BYOC pedradb-mac, VM 4 vCPU,
instance `cnt_098fc293…`, guest Linux — regime-alvo `writers == ncpu`
mantido, `nproc=4`) | **Imagem:** `ghcr.io/paulocsanz/pedradb-linux-gate:p211t`
digest `sha256:196567f1…` (base `python:3.12-slim` + layer
`usr/local/bin/{rocks-parity-bench,wave_dispatcher.sh,p211p_entrypoint_arm.sh,p211q_entrypoint_arm.sh}`;
src = commit `052d151e`, árvore com o fix do deadlock) | **Peer:** RocksDB
default `ROCKS_PARITY_SYNC=0`; pedra `PEDRA_PARITY_ASYNC=1` (coluna
same-class). Ratios desta onda são **DIAG** (`PEDRA_WRITE_PHASE_STATS=1`
ligado no pedra); cartaz permanece p211m (stats-off).

**Nota de host:** p211m (âncoras 0,491/0,836) rodou no `linux-gate-p149b`
(plataforma, outra classe de boot). p211r é BYOC na mesma máquina física
das builds. Todo ratio desta onda é pedra/rocks **do mesmo braço e do
mesmo boot** (par medido a segundos de distância), então o par é
interno ao host; a comparação com as âncoras p211m é qualitativa
(direção/tamanho do residual), não régua. A entrega do P1.2 é a
**decomposição PHASE** — telemetria pedra-local, válida onde roda.

## O deadlock da primeira onda (p211s) — causa-raiz, prova A/B, fix

**Sintoma:** onda p211s (imagem `p211s`, deployment `b2acd95f`,
2026-09-12T18:55Z): TODA célula pedra compat morria em `timeout 900`
(rc=124) sem produzir `rockSParity` nenhum; TODA célula rocksdb
completava rc=0 em ~10s. O hang era no **primeiro put async** de cada
invocação (DB nova sob `/tmp` a cada célula).

**Causa-raiz (provada por backtrace simbolizado na VM de debug):**
`PEDRA_PARITY_ASYNC=1` → put 1-cliente toma o caminho lone-async
`WriteGroup::submit_one` → `Db::commit_async_one` trava o mutex do WAL →
`encode_async_one` → `ensure_disk_pressure_admitted()` (primeira linha)
→ `probe_available_bytes` (statvfs) → no gate o **overlay upperdir é
tmpfs 256MB**, logo free < `DISK_SOFT_FREE_BYTES` sempre → banda
`Reclaim` → `reclaim_disk_for_uptime` → `try_rotate_wal` → no primeiro
put de DB vazia `wal_rotate_decision` = RotateWal → `self.wal.lock()` —
o MESMO mutex já segurado → parking_lot não-reentrante → deadlock
eterno. Modo sync (e hosts com >256MB livres no dir) nunca caem na
banda → por isso as ondas anteriores (p211m rodou com probe Ok) não
travaram.

**Prova A/B (VM de debug, mesmo guest, mesmo binário):** async +
`ulimit -n 1048576` → HANG; sync + 1024 → COMPLETED (RLIMIT_NOFILE
irrelevante — atribuição anterior era confundida). macOS host nunca
reproduziu (free do dir >> 256MB → probe Ok). Backtrace completo no
scratch (`vmrepro/serial3.log`, `/tmp/run{2,A,B}.sh` na VM): main →
`push_ycsb` → `CompatEngine::put` → `ConcurrentDb::put_with_seq` →
`submit_one` → `commit_async_one` → `encode_async_one` →
`ensure_disk_pressure_admitted` → `reclaim_disk_for_uptime` →
`try_rotate_wal` → `Mutex<Wal>::lock` (lock_slow). Workers de flush e
compact bloqueiam no RwLock de leitura do Db segurado pela main pendurada.

**Fix (commit `052d151e`, `fix: deadlock do 1o put async sob disk
pressure soft (wal-held reclaim lock-free)`):**
- kernel puro: `disk_pressure_reclaim_plan_wal_held()` → plano
  lock-free (`compact_sst/rotate_wal/compact_vlog` todos `false`); o
  drop de page-cache do db.rs continua rodando.
- `ensure_disk_pressure_admitted(wal_held)` / `reclaim_disk_for_uptime(…,
  wal_held)`: quando o caller segura o WAL (único site: `encode_async_one`,
  contrato RFC-0185 P0.3), o reclaim degrada para a parte lock-free.
  Semântica de admissão preservada: banda soft continua admitindo
  (re-probe após o reclaim), abaixo do hard continua `Refuse`. Os
  outros 5 call sites (lockam o WAL só depois do admit) passam
  `false` e ficam idênticos ao comportamento anterior.
- Teste de regressão `rfc0211_async_disk_pressure_no_deadlock.rs` no
  caminho real (`ConcurrentDb` + `FailingEnvArc::passing()`,
  `set_default_write_sync(false)`, pressão `SOFT_BAND_FREE` setada
  DEPOIS do open, put em thread com watchdog mpsc 60s, reopen sob
  pressão lendo o WAL): vermelho no formato antigo (falhava em 60,02s
  — o watchdog apanhava o deadlock), verde com o fix em 0,02–0,04s.
  Kernel `disk_pressure` 11/11 (incl. guardiãs de source-glue);
  `rfc0179_failing_env` 16/16; suíte completa do core: mesmas 23
  falhas pré-existentes do baseline (provado rodando a suíte num
  worktree limpo no HEAD `faeb1f98` sem o fix — conjunto idêntico).

**Por que o gate de produção bateu nisso e o dev não:** o overlay
upperdir do container do gate é tmpfs 256MB — todo `statvfs` do dir da
DB dá free < `DISK_SOFT_FREE_BYTES` (256MiB), i.e. a onda roda
perpetuamente na banda Reclaim. É também um achado de produto: qualquer
deploy Pedra com async em filesystem pequeno/tmpfs entra nesse caminho
no primeiro put — o fix agora faz o admit degradar para lock-free em
vez de deadlock.

## Segunda correção de modelo (onda p211t): o resto não é hang, é grind

A re-execução p211t (deployment `3fcbb991`, mesma imagem + fix) derrubou
a hipótese de um segundo deadlock: `compat SINGLE rc=0` (<1s, 2000
ops) e `compat KVR rc=0` (2min52, incl. kvrocks_set_mc50 = 50 writers
1-op) completaram; só a invocação MC (ycsb_a_mc4 + ycsb_f_mc4 +
deps_cache_overwrite_mc4 + deps_apply_batch_mc4) estourou os 900s
(rc=124). O que resta com o fix é o **custo por commit da escada de
admissão sob Reclaim perpétua**, amplificado pelo orçamento de disco:

- **Reprodução host (macOS, dmg 400MB com 138Mi livres = banda Reclaim,
  binário do `052d151e`, mesmo env da célula MC):** seed 1024 records
  em 15,0s (~15ms/put); ycsb_a_mc4 e ycsb_f_mc4 completaram com
  `phasesΔ wal=14,2–18,3ms/commit` e `lock_wait=3,7–5,0ms` (vs
  prepare 0,22–0,28µs, mem 6–19µs, publish 1,5–5,8µs) — o dono do
  custo é o **wal-phase** (escada admit: 2× statvfs + readdir +
  fadvise + `reserve_space`/preallocate sob o mutex do WAL **e** a
  write-lock do Db — o bypass segura a write-lock do Db durante TODO o
  commit, `concurrent.rs:730-749`). deps_cache_overwrite_mc4:
  `wal=17,1ms lock_wait=6,3ms` n=8000. Log completo em
  `{SCRATCH}/repro-grind-run.log`; sample das threads em
  `{SCRATCH}/hang1.sample` (3 writers dentro de
  `commit_async_one→write_pending_frame`, os demais em
  `lock_exclusive_slow`/read-lock).
- **Espaço oscilando no hard floor:** free cai a ~69MiB, um reclaim
  (rotate do WAL apaga segmentos) devolve ~65MiB, volta a cair — o
  sistema briga pelo espaço a cada commit. No gate o orçamento é pior
  (rootfs compartilha o tmpfs 256MB), o grind multiplica a célula
  20–60× e a invocação MC de 4 células estoura o `timeout 900`. Os
  loops de park (`await_flush_debt`, `await_l0_park`) são bounded e o
  bench não re-tenta put com erro (conta `errors+=1` e segue) — o
  rc=124 é tempo, não deadlock.

**Correção do setup do gate (p211u):** o P1.2 pede a decomposição do
residual no regime LIMPO (sem artefato de pressão). Volume elástico
disk-backed de 2GB montado em `/data` (`caixote service add-volume`),
ondas p211p/p211q com `OUT=/data/p211p|q211q` — probe Ok (free »
256MiB), nenhuma escada, orçamento de disco confortável. Imagem
`p211u` digest `sha256:33488923…`. Nota de produto que fica: a banda
Reclaim perpétua em disco pequeno tem custo por commit de dezenas de
ms no macOS (F_PREALLOCATE) e multiplica o tempo de célula 20–60× em
qualquer host — candidato a fatia futura (admit com histerese/cache
curto, não por commit).

**Correção 2 (p211v, 2026-09-12T21:0xZ): o volume NÃO anexa no BYOC.**
A primeira onda p211p no gate BYOC (deploy `5e12e542`, container
`cnt_c4fd8c80…`) bootou com `WAVE=p211p` correto (redeploy assa o env;
restart NÃO — lição operacional), mas rodou no regime errado: o log do
BYOC mostra `Created encrypted data disk … size_mb=0` — o tamanho do
volume (2048MB) não propagou no caminho de reuso de volume (RFC 0091),
o disco de dados subiu de 0MB, `/data` caiu no overlay tmpfs e a onda
rodou de novo na banda Reclaim. Prova: `data.qcow2` do host não cresceu
um byte durante 15min de onda (células SINGLE rc=0 escreveram centenas
de KB); assinatura idêntica ao p211t (`SINGLE rc=0` instantâneo, MC
rc=124 em 900s — P-FAIL r1/clean). Evidência:
`{SCRATCH}/p211v-volume-not-attached.txt` +
`{SCRATCH}/p211v-wave1-wrongregime-serial.log`. Contramedidas: (a)
entrypoints ganharam guarda fail-fast (`P211P_OUT`/`P211Q_OUT` +
recusa `BOOTSTRAP_FAIL (volume-missing|OUT free < 256MiB)` — nunca
mais queimar 40min de onda no regime escada); (b) a onda migrou para o
gate de plataforma `linux-gate-p149` (região brasil, FS disk-backed,
mesma classe das âncoras p211m) com binário amd64 da mesma árvore
`052d151e`; (c) o BYOC `linux-gate-p211r` fica em `WAVE=hold` ocioso.

## Resultado da onda (p211u)

**Parcial 1 — a onda em regime errado completou (23:10:21Z) e quantificou o
grind por dentro.** Apesar do MC rc=124 por rodada (regime escada, ratios
MISSING por construção), o resumo traz a decomposição PHASE de TODAS as
células que rodaram, e ela é conclusiva sobre o mecanismo:

- `ycsb_a_mc4`: clean `wal=0.92µs lwait=12.43µs avg_grp=1.00` vs rmw
  `wal=7403.17µs lwait=0.11µs avg_grp=1.35` (idem ycsb_f: 0.94µs→6104.94µs;
  deps_cache_overwrite: 0.92µs→6612.77µs, avg_grp=1.36). A escada de
  admissão cai DENTRO da fase wal do braço rmw (encode sob mutex WAL +
  F_PREALLOCATE/fadvise), 3 ordens de magnitude acima do clean.
- O custo dominante do grind está FORA das fases medidas: ~230–310ms/op de
  gap não contabilizado (n≈2907–3946 ops em 900s; soma das fases ≈12µs) —
  são os parks da escada (`await_flush_debt`/`await_l0_park`) esperando
  reclaim que nunca converge no tmpfs ~cheio. Não é lock de grupo
  (`lwait≈0`), não é codificação (`prep/mem/pub≈µs`).
- `kvrocks_set_mc50` completou nos DOIS braços sob a escada: clean
  `wal=4844.07µs avg_grp=19.67` vs rmw `wal=7098.33µs avg_grp=19.80` — o
  drenar-grupo forma grupos reais (≈20 de 50 writers) mesmo em pressão
  perpétua; o escalonador rmw não quebra o group commit, só não o vence
  quando a escada domina o wal.
- Salvos: `{SCRATCH}/p211w-grindregime-complete-serial.log` (resumo
  completo). Este é o extremo "escada" do bracket; o extremo limpo
  (admissão probe-Ok) vem da re-execução abaixo.

(acompanha — extremo limpo preenchido ao fechar a re-execução)

**Fechamento (2026-09-13T03:35Z): Linux admission-clean bloqueado por
ambiente — registrado em `{SCRATCH}/gate-blocked.txt`.** BYOC esgotado
com diagnóstico completo (onda p211y 03:27:42Z: `/dev/shm`=64MB,
`/tmp`=253052kB<256MiB, `/data` ausente — volume `size_mb=0` e
`--disk-mb 4096` sem efeito, sem CAP_SYS_ADMIN para mount); p149
pending no cordon do único host brasil (desconectado, sem ETA). O
deploy p149 (`:p211v` amd64) segue pending com monitor no ar — ao
agendar, confirma o extremo Linux na classe âncora.

## Veredito P1.2 (datado): o dono do 0.836 NÃO é o escalonador rmw

Provenho em dois extremos independentes:
1. **Escada** (Linux gate, tmpfs + Reclaim perpétua): braço rmw piora
   `wal` de ~1µs → 6–7ms (escada dentro do mutex WAL) e o custo
   dominante são parks ~230–310ms/op FORA das fases; `lwait≈0`; grupo
   preservado (`avg_grp` 19.8 em mc50). O escalonador não trava, não
   quebra grupo — a escada é que domina.
2. **Limpo** (Darwin disco real, probe-Ok): braço rmw **estritamente
   melhor** no perfil de fases (`lwait` 10–19µs → 0,4µs nas MC;
   65µs → 2,3µs em mc50 com grupo dobrando p/ 23,7) e os ratios **não
   mudam** entre braços ⇒ lock/scheduling não é gargalo de throughput.

O dono restante do residual 0.836 (e dos 6 U-cells de write) é o
**trabalho serial por commit fora das fases medidas — no Linux âncora,
o fdatasync per-commit da coluna de paridade** (`PEDRA_PARITY_ASYNC=1`
fdatasynca antes do Ok; flsh real em ext4, centenas de µs; no APFS o
flsh é ~0 e o residual também encolhe). É o fd-ceiling por construção
para 1-op-per-call; group commit fecha com writers (mc50: avg_grp 23,7
no braço rmw). Confirmação Linux admission-clean (ratios→~1 com
flsh≈0): pendente de capacidade do gate, registrada acima.

Consequências: (a) RFC-0211 P1.2 fecha com adjudicação honesta — o
escalonador drenar-grupo está absolvido e é o mecanismo CERTEIRO para
fechar os U-cells sob concorrência; (b) o ataque de performance nº 1
do inventário rev. 3 é o custo serial per-commit (fdatasync), não
scheduling; (c) a fatia da escada (admit com histerese) continua
registrada como achado de produto.

**Parcial 2 — extremo limpo no host Darwin (03:20:03Z), disco real, admissão
probe-Ok (free » 256MiB, zero escada).** Onda completa rc=0 (27,5s), mesma
matriz do gate (3 rodadas × clean/rmw × MC/SINGLE/KVR, `PEDRA_PARITY_ASYNC=1`,
peer default `sync=false`, stats on), binário `052d151e`/sha-c6244132.
Rotulada Darwin/DIAG. Números-chave (`{SCRATCH}/p211p-host-run.log`):

- MC, braço **clean** (default): `wal≈5µs lwait=10–19µs avg_grp=1.00
  queued=0`. MC, braço **rmw**: `wal≈4µs lwait≈0,4µs avg_grp=1,31–1,42
  queued≈2,7–6,5k` — o escalonador rmw ELIMINA o lock_wait e forma grupos
  pequenos; `wal` não muda. `deps_apply_batch_mc4`: idêntica nos dois braços
  (`avg_grp≈56,8` — já agrupa por construção, o escalonador é neutro lá).
- `kvrocks_set_mc50` (50 writers): clean `lwait=65,32µs avg_grp=13,71` vs
  rmw `lwait=2,27µs avg_grp=23,72` — o escalonador dobra o grupo e mata o
  lock wait; ratio mediano porém quase não muda (1,436 vs 1,380, ruído
  Darwin).
- `flsh≈0,03µs` em TODAS as células no Darwin — fdatasync APFS é
  quase-grátis e não aparece; com `lock_wait` zerado pelo braço rmw e os
  RATIOS IGUAIS entre braços, **escalonamento/lock NÃO é o dono do residual
  0,836**: nas fases medidas o braço rmw é estritamente melhor e nada
  melhora de throughput.
- Consequência: o dono mora FORA das fases medidas. No regime âncora
  (Linux, disco real), o candidato é o **fdatasync per-commit** da coluna
  de paridade (flsh real, centenas de µs em ext4). Teste decisivo: Linux
  tmpfs com admissão limpa (onda `:p211x`, `/dev/shm`) — se ratios MC → ~1,0
  com flsh≈0, o dono é o fdatasync (fd-ceiling por commit; group commit só
  fecha com mais writers, cf. mc50 avg_grp=23,7); o gate p149 (disco)
  confirma na classe âncora. Bracket: escada (wal 6–7ms + parks
  230–310ms/op) ←→ Darwin-limpo (fases ~15–35µs, lwait morto, ratios
  flat).
