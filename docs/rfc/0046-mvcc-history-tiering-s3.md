# RFC-0046: história MVCC fora do SSD — retention default + tier em object storage (S3)

**Status:** done (P0.1–P0.5 + P1 + P2.1–P2.8 — P0.4 fechado em
2026-08-22; registro em `findings/rfc0046-p04/`)
**Updated:** 2026-08-22
**Parents:** [0009](0009-rocksdb-class-engine.md) (F20 retention),
[0044](0044-async-class-5x-rocks.md) (E/cliff de retenção),
[0045](0045-multi-writer-async-5x.md)
[AGENTS.md](../../AGENTS.md) (colunas oficiais medem o default do produto)

## Background

- Hoje o default do produto **retém todas as versões** (F20): MVCC,
  change-feed (`changes(from,to]`) e PITR por seq são de graça *em
  capacidade*, mas o **custo de storage cai no SSD local** — disco cresce
  ~O(volume total de writes), não O(live set). Medido no próprio bench:
  2M ops sobre 1024 chaves (~1 GB lógico) viraram 2 SSTs de 72 MB + WAL de
  180 MB, enquanto o Rocks default compacta para ≈ live set.
- O cliff do E (RFC-0044 P2.2) é a mesma doença em leitura: scan frio é
  O(versões retidas). `auto_reclaim` (opt-in, d9abd6f) já existe como
  mecanismo de GC pin-aware; `compact_for_reads` como operação manual;
  checkpoint/backup com verify, PITR por seq e ship-wal já existem.
- Concorrência: toda plataforma séria mora o PITR em storage barato
  (WAL archive em S3 no Postgres/cloud; snapshot+WAL em S3 no
  MongoDB/Atlas; TiDB PITR em S3/NFS) e mantém o disco local ≈ live set +
  janela curta. Cassandra tem precedente de **retenção default**
  (`gc_grace_seconds` = 10 dias): o default do produto é bounded, não
  "tudo para sempre" *(conhecimento estabelecido, não fonte primária
  desta sessão)*.
- Sem S3 habilitado, o produto atual é **insustentável em prod** (disco
  cresce sem borne). Este RFC fecha os dois casos: sem S3 (retenção
  bounded + arquivo local com cap) e com S3 (tier de história em object
  storage, PITR de lá).

## Problems This Solves

- **Problem:** disco cresce O(writes) com retention default — não
  sustentável sem operação manual; "PITR de graça" custa SSD caro.
- **Problem:** scan frio O(versões) (cliff do E) é a face de leitura da
  mesma decisão; retention bounded fecha a classe estruturalmente, não
  só via cache.
- **Problem:** histórico que sai do SSD hoje não tem destino barato; PITR
  depende do que sobrou no disco local.
- **Problem:** a história só é restaurável se houver backup rodando —
  hoje é disciplina do operador, não produto.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G2 | CRC fail-closed / never silent-wrong | tier e archive entram no verify; GC nunca destrói silenciosamente |
| GC-fail-closed | watermark + `SnapshotTooOld` (erro tipificado) | retention bounded **é** o watermark movendo; pins respeitados |
| F20 | retenção total possível | continua existindo — vira **opt-in explícito**, não default |
| G1 | `fdatasync` antes do Ok | intocada |
| G6 | sem thread no core | upload/tier roda no host (compat/montanha), não no core |
| — | shape não some (0043) | catálogo intocado; colunas oficiais passam a medir o **novo default** (P0.4 re-árbitro) |

## Proposed Solution

1. **Retention default bounded**: `OpenOptions::history_horizon` —
   `HistoryHorizon::All` (F20, opt-in explícito) ou
   `HistoryHorizon::Window(d)` (default do produto; `d` decidido no P0 com
   remesura — strawman 24 h). O auto-compact GC'ia versões mais velhas que
   o horizonte **depois de arquivá-las** (P0.2), pin-aware como o
   `auto_reclaim` atual. SSD = live set + janela.
2. **Arquivo local bounded** (funciona **sem S3**): antes do GC, as
   versões saem para um archive local (ship-wal/history SST) com cap e
   rotação — PITR local por seq continua, limitado pelo cap; estouro do
   cap avança o watermark (o mais velho vira `SnapshotTooOld`), nunca
   destrói silenciosamente.
3. **Tier em object storage**: destino alternativo/adicional do archive —
   S3-class via seam `Env` (bytes content-addressed + CRC), upload no
   host, retry/resume de crash. PITR restaura do tier (restore-time), como
   o mercado faz.
4. **Leitura do tier é P2** (lazy): P1 é restore-only — igual aos
   concorrentes.

## Delivery slices (mandatory)

### P0 — retention sustentável sem S3

- [x] **P0.1** `OpenOptions::history` (`HistoryOptions { horizon, cap_bytes }`,
      `HistoryHorizon::All | Window(Duration)`) + default do produto
      **`Window(24 h)` com cap 1 GiB** (24 h: janela de PITR "de graça" no
      SSD sem replicar o `gc_grace` de 10 d do Cassandra — disco ≈ live set +
      janela; cap 1 GiB borneia o archive local sem S3); F20 vira `All`
      explícito; sampling seq×tempo a cada 32 publishes (`Env::unix_millis`
      seam, determinístico em teste); doc de produto: `docs/usage.md`
      §"History retention — bounded by default". Nota bench: runs curtos
      (< 24 h de wall clock) nunca envelhecem amostras além da janela → cutoff 0
      → comportamento F20 dentro do bench; colunas oficiais só mudam no
      re-árbitro P0.4 com janela longa. **Hardening pós-review (mesma
      data): cap duro no ring de amostras** — a regra temporal (2× janela)
      limita o *span*, não a *contagem*: janela longa sob escrita
      sustentada crescia a memória sem bound (1 amostra/32 publishes; 24 h
      a 10k w/s ≈ 54 M amostras ≈ 864 MB) e o scan do cutoff era O(ring)
      por flush. `HORIZON_SAMPLE_RING_CAP = 4096` (granularidade do cutoff
      vira janela/4096 ≈ 21 s — ruído contra 24 h; drop-oldest só atrasa o
      cutoff, nunca adianta) + drain das amostras consumidas no próprio
      cutoff; `history_stats().seq_time_samples` expõe o tamanho. Teste
      `horizon_sample_ring_hard_capped_and_drained` — status: `done`
- [x] **P0.2** Archive local antes do GC: `history/seg-*.hist` (streaming,
      8192 records/segmento, CRC32c por record), `history/MANIFEST` (magic
      PHST, tmp+rename, manifest-is-truth — crash deixa no máximo arquivo
      não referenciado, removido no próximo open); cap + rotação (estouro
      derruba segmentos mais velhos e avança o watermark com
      `SnapshotTooOld` tipificado — nunca destrói silenciosamente; pin
      segura o segmento); GC pin-aware reutiliza
      `CompactGcOptions::for_oldest_snapshot`; **fail-closed**: erro de
      archive pula a rodada de GC (merge history-preserving). Tier lazy:
      `history/` só materializa no primeiro archive (dir de DB/checkpoint
      restaurado continua flat). **Perfis separados**: `auto_reclaim=true`
      é reclaim puro **sem archive** (perfil Rocks da face compat,
      RFC-0047 divergência 4 — o teste `auto_reclaim_default_matches_rocks_profile`
      pegou o over-archive e fechou); archive só quando o floor vem do
      horizon — status: `done`
      (**caveat pós-telemetria**: "disco ≈ live set + janela + cap" vale
      para o archive/, não para o total — **fechado pelo P0.5**, que
      limita o LSM com o trigger de dead-weight-doubling)
- [x] **P0.3** Testes: `snapshot_pinned_survives_horizon` (pin sobrevive a
      aging+GC; release + novo envelhecimento → `SnapshotTooOld`);
      `pitr_local_by_seq_within_window`; `archive_cap_overflow_advances_watermark_not_silent`
      (watermark avança, latest intacto, pin segura, bytes ≤ cap);
      `archive_crash_mid_upload_reopens_consistent` (fault `.hist` → GC
      pulada, história intacta, reopen consistente);
      `history_horizon_all_keeps_all_versions` (F20 opt-in re-verde).
      Duas regressões reais pegas pela bateria e fechadas: backup/restore
      quebrava com o novo subdir (`copy_db_directory` tratava diretório
      como arquivo — `metadata_len` é `Ok` em dir no macOS; agora
      `Env::is_dir` pula dir de verdade) e o over-archive do perfil
      Rocks (acima) — status: `done`
- [x] **P0.4** Re-árbitro quieto 3× com o novo default (colunas oficiais
      0041 medem o default do produto): E/scan/A–D + regressão G1;
      expectativa: cliff do E some estruturalmente (retenção), CountCache
      segue no caminho quente — status: `done`
      (`findings/rfc0046-p04/`, 2026-08-22). Três execuções: **v1
      descartada** (rodadas a n=200 — default de fumaça do bench; bug
      do script, fix `27b1fd7`), **v2** disparou limpo (load 8,9–9,95,
      n=2000) mas **mediu a árvore de trabalho suja** (edits
      não-commitados de uma sessão paralela em occ/memtable/wal), e a
      **clean/** (worktree em `edfa132`, mesmo box, mesma barra) é o
      registro oficial. **Veredito: default novo e todo o arco
      RFC-0046 commitado não custam nada mensurável** — controle
      decisivo: a árvore do p34 (`04c7aa2`) rodando hoje é igual à
      `edfa132` em toda shape (compat estável no arco inteiro; e o
      default é inerte em bench por construção — janela wall-clock,
      processo curto não GC nada). Colunas (med-de-3, peer Rocks
      default `sync=false`): g1 c 4,67 / e 2,17 / scan 2,04 (a/f no
      teto de fdatasync conhecido, 0041 P1.2 `todo`); async E 5,88
      (3/3 ≥5) / a 2,98 / f 1,40 / lock 2,54; longa 2M 3×: **set 5,45
      (3/3 ≥5)**, pipeline 5,38 (med ≥5; min 4,95), get 4,71 (3/3
      tight <5 — straddle do GET é real, não load). Bônus forense
      (README do finding): os edits da sessão paralela movem o compat
      (a −2,5×, f −1,7× — caminho RMW; e +1,8×) e o peer rocks mediu
      ~1,8× mais rápido que 20/ago (drift de caixa, tight 3/3 em ambas
      as eras).
- [x] **P0.5** Rewrite de níveis velhos dirigido pelo horizonte (nascido da
      telemetria `findings/rfc0046-sizing/`, 2026-08-21): **o caminho
      default não devolvia ao disco o que envelheceu** — o floor do
      horizonte sempre atrasa os inputs do `compact_l0_into_l1` (versões
      cruzam a janela depois de chegar a L1, e L1 nunca é reescrito).
      Medido no workload corrigido (256 chaves estáveis × 40 overwrites):
      sem o fix, `all` vs `window` byte-idênticos (LSM 10 991 231 B nos
      dois) e o window ainda arquiva cópia por cima — PIOR que F20;
      `auto_compact_sst_count` não salva (promove um nível por vez, L0
      vence). Erratum: a primeira rodada da telemetria media um workload
      sem overwrite (sufixo de chave re-sortido por round — zero versões
      repetidas); a conclusão sobreviveu, a evidência não — ver
      `findings/rfc0046-sizing/` (README erratum + contrafactual).
      Fix entregue: **dead-weight-doubling trigger** em
      `maybe_auto_compact` — quando o floor avançou além do último full
      reclaim E os bytes de SST pelo menos dobraram desde então,
      reescrever TODOS os SSTs com o GC floor (archive-first,
      fail-closed; erro de archive retenta no próximo flush). Máx. uma
      reescrita por dobramento de peso morto; `last_horizon_reclaim`
      in-memory (reopen pode pagar uma extra, auto-limitante). A/B:
      LSM 10 991 231 → 2 710 428 B (4,1×), archive no cap, total
      0,64× escrito. `auto_reclaim` inalterado (floor maximal).
      Teste `horizon_full_rewrite_bounds_disk` (bound + latest intacto
      + leitura abaixo do watermark pelo archive). **API pública
      `Db::compact_horizon()`**: a mesma reescrita sob demanda do operador
      (flush → archive-first fail-closed → rewrite ALL sob o floor; no-op
      em `All`; renova a baseline do trigger), teste
      `compact_horizon_reclaims_aged_versions`. Wart (watermark global ×
      sobrevivência por chave): **fechado pelo P2.3** (fallback LSM) —
      status: `done`

### P1 — tier S3 (história barata e PITR de lá)

- [x] **P1.1** Destino object storage no seam `Env` — `history::RemoteTier`
      (superfície **pública** p/ o host; G6: upload roda no host, o core só
      dá o seam): objetos imutáveis content-addressed
      `seg-<len>-<crc32c>.hist` com dedup **verificada por read-back**
      (colisão de nome = erro tipado `CorruptHistory`, nunca bytes errados
      silenciosos); segmento é walkado e CRC-verificado por record **antes**
      de sair da máquina; manifestos como gerações imutáveis
      `MANIFEST-<n>` + ponteiro `LATEST` reescrito por upload (object
      store não tem rename; LATEST torcido → walk-back para a geração
      íntegra mais nova); puts idempotentes (base do retry/resume do
      P1.2). Testes sem rede (`MapEnv` em memória + `FaultyEnv` de
      create): endereçamento+idempotência, recusa de segmento local
      corrupto (nada sobe), colisão fail-closed, gerações+walk-back,
      falha de I/O sem objeto parcial + retry completa, round-trip de
      leitura — status: `done` (cc760e5)
- [x] **P1.2** Pipeline de upload: `Db::set_remote_history(env, root)`
      (opt-in do host; uploads rodam inline no caminho do auto-compact —
      sem thread nova no core, G6) — a cada rodada de archive:
      segmentos selados → `put_segment` idempotente (retry/resume grátis
      por content-addressing), depois a geração de manifesto;
      **backpressure**: tier caído → a rodada de GC inteira pausa
      (merge history-preserving) e o cap local **segura** segmentos não
      subidos (disco local cresce — tradeoff documentado; nunca destrói o
      que não subiu); verify de bytes no destino = read-back CRC do P1.1;
      `upload_history_now()` expõe o passo p/ o host/CLI. Testes:
      outage pausa GC (earliest parado) e recupera; cap segura não-subidos
      durante outage e libera após upload; resume cross-reopen é
      idempotente (AlreadyPresent, nada re-escrito) — status: `done`
- [x] **P1.3** Restore drill do tier: `pedradb_ops::restore_history_from_remote`
      (manifesto v2 carrega o nome content-addressed de cada segmento —
      `RemoteTier::latest_segments`); lê o manifesto íntegro mais novo,
      faz stream de cada segmento com CRC por record verificado no replay,
      e regrava as versões ≤ target num WAL novo **em ordem de seq** —
      a recuperação existente reconstrói o DB com as sequências originais.
      Semântica honesta: o tier restaura o **prefixo arquivado** (versões
      que envelheceram além do horizonte); a cauda in-window é do
      WAL-ship (`restore_with_increments`) — o mesmo split base+WAL do
      PITR do Postgres. E2e `pitr_restore_from_object_storage`: destrói o
      local, restaura full (estado no cutoff do archive + MVCC em seq
      arbitrária), restaura em seq 20 exato, e bytes remotos corrompidos
      → erro tipado, nunca restore silencioso errado — status: `done`
- [x] **P1.4** `pedra` CLI: `pedra archive status <remote_root>` (roll-up
      do manifesto íntegro mais novo: segmentos, bytes, range de seq,
      archive floor, próxima geração; vazio/lixo → mensagem graciosa,
      walk-back do LATEST se aplica) e `pedra archive restore
      <remote_root> <dest> [target_seq]` (drill P1.3 do shell; sem seq =
      prefixo arquivado completo) — status: `done`

### P2 — polish

- [x] **P2.1** Leitura lazy do tier (snapshot mais velho que a janela
      local servido do tier, não só restore) — `get_at` abaixo do
      watermark cai para o tier: segmentos locais primeiro, espelho
      remoto segundo (o remoto retém o que o cap local já derrubou).
      Registro decisivo (put/delete/range-delete mais novo com
      `seq ≤ snap`) responde mesmo com buracos de cobertura; sem
      registro, `None` só quando a cobertura retida prova cobrir
      `[1, snap]` sem drops (never-written vs dropped são
      indistinguíveis — fail-closed `SnapshotTooOld`). Custo v0: cada
      segmento retido é CRC-walked por leitura (sem índice de chaves
      ainda); scans continuam fail-closed. Bytes remotos corrompidos →
      `CorruptHistory` tipado — status: `done`
- [x] **P2.2** Métricas (bytes locais vs tier, idade do archive, uploads
      pendentes) + limiter de banda — `history_stats()` roll-up
      (segmentos/bytes locais, archive floor, watermark, pending
      uploads, sumário remoto, idade da última passada de archive);
      `set_upload_bandwidth(bytes_por_rodada)` limita bytes enviados
      por upload step e **não** envia manifesto enquanto faltam
      segmentos no destino (o remoto nunca lista o que não tem);
      backlog drena idempotente, cap segura o não-enviado — status:
      `done`
- [x] **P2.3** Fallback LSM na leitura abaixo do watermark (wart do P0.5,
      fechado): o watermark é **global** mas a sobrevivência é **por
      chave** — chaves de versão única abaixo do floor sobrevivem ao
      rewrite, e o cap pode derrubar o segmento del do archive; a leitura
      em `seq < watermark` ia só ao tier e falhava `SnapshotTooOld` com a
      versão viva no LSM (perda de disponibilidade, fail-closed). Fix
      sound-undo: quando o tier não cobre a leitura
      (`SnapshotTooOld` de cobertura), cair para o LSM e servir
      **somente** registro decisivo fisicamente presente a `seq ≤ snap`
      (`get_at_below_watermark_lsm`); `NotFound` no LSM mantém o erro do
      tier — nunca-escrita não é provável lá (todas as versões da chave
      podem ter sido GC'd e tombstone-cleaned), então `None` ali seria
      destroy silencioso. Teste
      `below_watermark_lsm_fallback_serves_survivors` (sobrevivente
      responde pós-cap-drop; sombra e never-written seguem
      `SnapshotTooOld`; verificado que o teste falha sem o fix).
      **Unificação P2.3b (mesma data): `multi_get_at` delega em
      `get_at`** — antes ele tinha só o check imediato
      (`ensure_snapshot_readable`) e errava abaixo do watermark mesmo
      quando o tier cobria a leitura que `get_at` responderia (fonte de
      disponibilidade divergente entre APIs com o mesmo contrato); agora
      compartilha as pernas tier+LSM e um lote com chave incobrível
      falha fechado como um todo — status: `done`
- [x] **P2.4** Change feed fail-closed abaixo do watermark (audit pós-P2.3):
      `changes(from, to]` retornava `Ok` com o que sobreviveu ao GC —
      eventos intermediários e **tombstones solitários** somem de todas
      as fontes (WAL rotacionado, cache de changelog, rebuild do LSM
      pós-GC) = partial silencioso. Fix: window começando abaixo do
      watermark → erro tipado `SnapshotTooOld` (mesma régua de
      `get_at`/`try_scan_at`); da watermark em diante o tail é exato.
      `changes_after` (semente last-write-wins de fold/journal) segue
      `Vec` sem erro e **está correto sob GC** para essa semântica (a
      versão mais nova por chave sempre sobrevive; tombstone dropado =
      chave ausente = mesmo estado final) — nuance documentada.
      Teste `changes_feed_fails_closed_below_watermark` — status:
      `done`
- [x] **P2.5** Índice por segmento do archive — **entregue no escopo
      key-range** (manifesto v3, back-compat v2): cada segmento selado
      carrega a cobertura `[key_lo, key_hi]` no manifesto, com teto
      **consciente de range-delete** (o fim exclusivo do RD conta para o
      teto — senão a poda seria insound); `get_at_from_archive` pula
      segmento local cuja cobertura exclui a chave **sem tocar no
      arquivo** (observável: segmento fora do range corrompido não
      atrapalha a leitura; dentro do range segue fail-closed CRC).
      Listagens remotas não carregam cobertura (walk completo — o
      espelho é o caminho de exceção). CRC por record inalterado.
      **Poda não ajuda quando os ranges sobrepõem** (workload de
      overwrite: todos os segmentos cobrem o mesmo key set) — a
      refinamento para sobreposição (bloom por segmento ou amostra
      esparso 1/64 no seal) fica como P2.6. Testes:
      `segment_key_coverage_prunes_reads_soundly` (bound + observável
      de poda + fail-closed preservado),
      `segment_key_coverage_counts_range_delete_ends` (soundness do
      teto), `manifest_v2_decodes_without_key_coverage` (back-compat) —
      status: `done`
- [x] **P2.6** Bloom por segmento do archive — **entregue como sidecar**
      `history/seg-<id>.bloom` (não no manifesto: com 1 GiB de cap e
      segmentos de 128 KB são ~8k segmentos; blooms no manifesto o
      inflariam para dezenas de MB): reusa o `BloomFilter` do kernel
      (~10 bits/key, ~1% FP), escrito e fsyncado **antes** do manifesto
      persistir no seal (manifesto implica sidecar durável; crash no
      meio = sidecar órfão, removido no open junto dos `.hist` órfãos;
      cap drop remove o sidecar junto). Range deletes não entram no
      bloom (intervalo não é enumerável) — viajam como **intervalos
      explícitos** no sidecar: chave é podável só com bloom negativo E
      nenhum intervalo cobrindo (sound). **Fail-open por construção**:
      sidecar ausente (segmento pré-P2.6), corrompido (CRC do corpo) ou
      truncado → leitura como "pode afetar" (walk) — filtro danificado
      nunca poda. Falso-positivo = walk (só mais lento). Espelho remoto
      segue walk (sem sidecar no objeto). Layout:
      `PHB1|ver|bloom.encode()|rd_count|intervals|body_len|crc32c(body)`.
      A leitura pula o walk do segmento (MBs) lendo só o sidecar
      (~10 KB); a prova de cobertura `None` continua usando a lista de
      candidatos (spans de seq) — skips não afetam disponibilidade.
      Testes: `bloom_sidecar_prunes_overlapping_key_ranges` (o caso que o
      P2.5 não poda), `bloom_sidecar_range_delete_intervals_sound`,
      `bloom_sidecar_missing_or_corrupt_never_prunes` (fail-open +
      cleanup de órfão) — status: `done`. **Medido**
      (`findings/rfc0046-p26/`, exemplo `rfc0046_p26_bloom_ab`): o caso
      pontual que só o P2.6 poda é a chave nunca escrita dentro do
      coverage de todo segmento (o stream é ordenado por chave — a prova
      de `None` precisa de todos os candidatos): 41 segmentos, perna
      sidecar 39,5 µs/read vs walk 2 169 µs/read (**55×**; bytes 331 KiB
      vs 23,4 MiB por leitura; sidecar = 1,4% do archive), caixa suja
      load≈41 — a razão é a alegação; ambas as pernas responderam `None`
      (cross-check do fail-open).

- [x] **P2.7** Índice de leitura do espelho remoto — fecha a lacuna que o
      P2.6 deixou aberta ("espelho remoto segue walk"): a perna remota
      do `get_at_from_archive` ainda baixava o objeto inteiro para
      decidir. Duas podas, mesmas semânticas das pernas locais:
      (a) **bound do manifesto v3**: `RemoteSegment` carrega
      `key_lo/key_hi` (`None` = manifesto remoto pré-P2.5 → walk,
      back-compat); segmento cujo range não contém a chave não é baixado.
      (b) **sidecar no objeto**: `put_segment` embarca o
      `seg-<nome>.bloom` junto (idempotente por read-back len+crc;
      divergência = `CorruptHistory` fail-closed); a perna
      local-absente consulta `read_sidecar` **antes** do fetch — bloom
      negativo + nenhum intervalo rd cobrindo pula o download. Fail-open
      idêntico ao P2.6: sidecar ausente (upload pré-P2.7) ou corrompido
      → fetch+walk. **Correção real no caminho**: colisão no nome
      content-addressed — workload estruturado de 3 ondas (mesmo key
      set, seqs deslocadas) selou dois segmentos com mesmo len E mesmo
      crc32c; o upload falhava fechado na verificação read-back
      (fail-closed correto, mas erro forçado em workload legítimo).
      Nome agora 3-digest `seg-<len:016x>-<crc32c:08x>-<fnv1a64:016x>.hist`;
      objetos 2-digest já presentes coexistem (o nome vem do manifesto)
      e a verificação read-back continua valendo. Testes:
      `remote_manifest_bound_prunes_fetch` (objetos a-only corrompidos
      nunca baixados para leitura de b-key, mesmo sem sidecar; a-key
      → `CorruptHistory` = controle), `remote_sidecar_prunes_segment_fetch`
      (buraco nunca escrito responde sem tocar objetos corrompidos;
      sem sidecars o mesmo read → `CorruptHistory` = prova do
      fail-open), `remote_upload_ships_sidecars_idempotently`
      (2 passes: 0 uploads novos, 1 sidecar por segmento),
      `remote_segment_put_content_addressed_and_idempotent` estendido
      (sidecar embarca exatamente 1×) — status: `done`. Wart conhecido
      restante: segmento remoto ainda faz walk CRC quando é baixado
      (sem cache de blocos remoto) — aceitável no v0.

- [x] **P2.8** Cache de leitura remota — fecha o wart que o P2.7
      registrou ("segmento baixado ainda faz fetch+walk a cada
      leitura"): LRU em memória, limitado por bytes (default **64 MiB**,
      `set_remote_read_cache`; `0` desliga), guardando o segmento já
      **decodificado e verificado** — hit pula o fetch E o walk
      (parse+CRC). **Soundness**: a chave do cache é o nome
      content-addressed de 3 digests (P2.7) — um nome identifica os
      bytes; a entrada só entra depois do walk CRC completo, então
      hit = bytes já verificados. Contrato verify-once documentado:
      bytes trocados sob um nome existente (corrupção em nível de
      operador) não são redetectados após a primeira leitura verificada
      daquele segmento — o cache confia no nome; com budget 0 toda
      leitura refaz fetch+walk e a corrupção fecha tipada como antes.
      Entradas maiores que o budget nunca entram; cortar o budget
      ejeta na hora (LRU). Telemetria: `history_stats()` ganha
      `remote_cache_entries`/`remote_cache_bytes`. Testes:
      `remote_read_cache_trusts_verified_name` (objetos corrompidos
      após o cache: leitura responde da cópia verificada; budget 0 →
      `CorruptHistory` = controle pre-P2.8),
      `remote_read_cache_oversize_never_caches` (budget 1: nada entra,
      corrupção fecha na leitura seguinte),
      `remote_read_cache_budget_cut_evicts` (corte ejeta; refetch
      reencontra a corrupção). Oráculo dos testes é discriminação de
      erro: cacheado = resposta limpa, refetch = `CorruptHistory`
      (propaga antes do fallback LSM do P2.3 — o LSM pode servir o
      registro fisicamente presente, então valor sozinho não
      discrimina). **Medido** (`findings/rfc0046-p28/`, exemplo
      `rfc0046_p28_remote_cache_ab`): leitura decisiva abaixo do
      watermark de segmento remote-only (bloom-positivo — o caso que
      nenhuma poda resolve), melhor de 3 fases interleaved: cached
      5 039,5 µs/read vs refetch 9 807,1 µs/read — **1,9× end-to-end**.
      As duas pernas pagam o mesmo piso de ~5 ms de walk nos segmentos
      LOCAIS may-affect (bloom-positivos, versões acima do snap); o
      cache elimina por completo o fetch+walk do objeto remoto (delta
      de ~4,8 ms/read — num object store real é o round-trip + egress
      de CADA leitura). Caixa suja (load ~12); a razão é a alegação.
      Wart remanescente menor: o hit ainda varre os registros do
      segmento (`decide_at` linear; registros ordenados por chave —
      busca binária fica como follow-up).

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | history_horizon + default bounded | **done** | b68f9a1 (+docs neste commit) | 2026-08-21 |
| P0.2 | p0 | archive local bounded + GC pin-aware | **done** | b68f9a1 (+docs neste commit) | 2026-08-21 |
| P0.3 | p0 | testes pin/cap/crash/PITR local | **done** | b68f9a1 (+docs neste commit) | 2026-08-21 |
| P0.4 | p0 | re-árbitro quieto com novo default | **done** | clean/ edfa132: 04c7aa2≡edfa132, default inerte; v1 descartado (n=200), v2 dirty = evidência de anomalia (p04) | 2026-08-22 |
| P0.5 | p0 | rewrite de níveis velhos pelo horizonte (LSM bound) | **done** | dead-weight-doubling trigger + teste; erratum rfc0046-sizing | 2026-08-21 |
| P1.1 | p1 | Env→S3 + testes seam | **done** | cc760e5 | 2026-08-21 |
| P1.2 | p1 | upload pipeline + backpressure | **done** | b548a5e | 2026-08-21 |
| P1.3 | p1 | restore drill do tier | **done** | c429acb | 2026-08-21 |
| P1.4 | p1 | CLI archive/restore | **done** | 751e9d4 | 2026-08-21 |
| P2.1 | p2 | leitura lazy do tier | **done** | `eea769d` | 2026-08-21 |
| P2.2 | p2 | métricas + banda | **done** | `eea769d` | 2026-08-21 |
| P2.3 | p2 | fallback LSM abaixo do watermark (wart cap×sobrevivente) | **done** | `get_at_below_watermark_lsm` + teste | 2026-08-21 |
| P2.4 | p2 | change feed fail-closed abaixo do watermark | **done** | `changes` check + teste | 2026-08-21 |
| P2.5 | p2 | índice por segmento do archive (custo de leitura) | **done** | manifesto v3 key-range rd-aware + 3 testes | 2026-08-21 |
| P2.6 | p2 | bloom por segmento (ranges sobrepostos) | **done** | sidecar `seg-*.bloom` fail-open + 3 testes + A/B 55× (p26) | 2026-08-21 |
| P2.7 | p2 | índice de leitura do espelho remoto | **done** | bound v3 + sidecar no objeto + fix colisão de nome (3-digest) + 4 testes | 2026-08-22 |
| P2.8 | p2 | cache de leitura remota (LRU por bytes) | **done** | 64 MiB default + `set_remote_read_cache` + 3 testes + A/B 1,9× e2e / fetch zerado (p28) | 2026-08-22 |

## Acceptance Criteria

- **Tests:** `snapshot_pinned_survives_horizon`;
  `archive_cap_overflow_advances_watermark_not_silent`;
  `archive_crash_mid_upload_reopens_consistent`;
  `pitr_restore_from_object_storage` (MemEnv-backed e2e);
  `history_horizon_all_keeps_all_versions` (F20 opt-in re-verde);
  suítes adversariais existentes sem editar asserção.
- **Telemetry:** `findings/rfc0046-*/` com o re-árbitro do P0.4 (quieto
  3×, colunas oficiais) + sizing de disco antes/depois (live set + janela)
  num workload de overwrite.
- **Documentation:** este RFC; `docs/usage.md` (novo default);
  `docs/certainty-vs-availability.md` (requisitos operacionais por
  garantia); RFC-0044 P2.2 nota (cliff fecha por retention, não só cache).
- **Screenshots:** none — backend-only.

## Out of scope

- Mudar G1 ou o peer oficial (0041).
- Object-store-first engine (não-goal permanente do `positioning.md`; o
  tier é *destino de história*, não substrato do kernel).
- Replicação/multi-node (Montanha).
- Lazy read do tier antes do P2 (P1 é restore-only, como o mercado).
