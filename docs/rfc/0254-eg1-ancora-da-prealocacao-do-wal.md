# RFC: A pré-alocação do WAL cobre a fronteira de escrita, não o tamanho lógico

**Status:** done (engineering win, no ladder pay)
**Updated:** 2026-09-23

## Background
- EG1 62%. Escada impaga: A4 (`deps_cache_overwrite_mc4` 25M, melhor
  mediana válida 0,7175 `:p252`, 0,7038 `:p254`), A5 (`ycsb_f_mc4`,
  mínimo 0,9206 `:p253`), C2 (`fjall_seq_1m`, fjall colapsado 6 rodadas).
- Diagnóstico `:p254` (diag2/3/4, sem env de diag nas rodadas pagantes):
  a fase `wal` do A4 paga **1,25–1,32 µs/commit no estado 25M** contra
  **0,19 µs** no estado 5M. `lock_wait` dobra junto (18,83 µs vs
  8,86 µs por commit) — a seção crítica cresceu e 4 escritores solo
  (bypass 0201, `avg_group=1.00`) amplificam ~4×.
- Sampler de syscall a 1 kHz com timestamp (`/data2/diag4/`): o
  `fallocate` (syscall 285) tem 36.952 amostras de dwell no 25M-mmap
  contra 1.962 no 5M — **todo na fase seed** (janela ops: zero 285,
  zero `pwrite64`; o dwel da ops é `futex` + `running`). A fase ops de
  25M não faz syscall de escrita nenhuma e ainda paga 1,25 µs na cópia
  mmap.
- Leitura do código fecha a cadeia causal:
  1. `preallocate_file` (posix) faz `fallocate(KEEP_SIZE, offset =
     i_size, len)` — ancora no **tamanho lógico**.
  2. A janela mmap do RFC-0253 (`mapped_frame_write`) mantém o arquivo
     com `ftruncate` até o topo da janela: `i_size` anda até 64 MiB
     **à frente** de `pos`. A região `(pos, i_size)` que o escritor
     vai preencher é **buraco esparso** — o `fallocate` aloca é a
     faixa `[i_size, i_size+64M)`, muito adiante da fronteira.
  3. `reserve_space` avança `prealloc_to` mesmo quando a 2ª chamada é
     no-op (região já alocada): a fronteira reivindicada **deriva** e
     as reservas param de disparar no fim do seed (janela
     t−40..t−20: zero fallocate) — o crescimento todo vira buraco.
  4. Escrever via mmap em página de buraco = minor fault por página +
     delayed allocation no writeback. No 25M (DB de vários GiB num
     guest de 4 GiB) a pressão desmaia/refaulta páginas da janela:
     majflt 0 (diag3) e minflt por op — o µs extra da fase wal. O
     25M-pwrite (`PEDRA_WAL_MMAP=0`) paga syscall (~1,26 µs) e mesmo
     assim é o 25M mais rápido (159,8k vs 140,5k com sampler ligado).
- O alvo limpo da onda `:p254`: A4 pedra ~205k mediana vs rocks ~257k
  (precisa +25%). Tirar ~1 µs da seção crítica tira ~4–5 µs do
  per-op sob contenção — é o tamanho do corte.

## Problems This Solves
- **Problem:** a reserva de espaço do WAL aloca a faixa errada (à
  frente do `i_size` arrastado pelo `ftruncate` da janela mmap) e a
  contabilidade deriva; o 25M escreve em buraco esparso e paga fault
  por página dentro da write lock, amplificado pela contenção.

## Proposed Solution
- `preallocate_file(file, offset, len)`: reserva `[offset, offset+len)`
  com `FALLOC_FL_KEEP_SIZE` (mesmo nome de fn — allowlist do posix não
  muda). Darwin mantém `F_PEOFPOSMODE` (offset é best-effort lá).
- `EnvFile::preallocate_at(offset, len)` (default no-op, como
  `preallocate`): impls de produção repassam (StdEnv, IoUringFile,
  wenv, store). O vlog continua no `preallocate(len)` — sem mmap, o
  âncora `i_size` do vlog é a fronteira correta.
- `reserve_space` reserva **exatamente** `[prealloc_to,
  need alinhado a 64 MiB)`: uma chamada por chunk, ancorada na
  fronteira real de escrita, sem deriva. O buraco `(pos, i_size)`
  criado pelo `ftruncate` fica coberto porque `need ≥ i_size`.
- Semântica de recuperação inalterada: `KEEP_SIZE` não mexe no
  tamanho lógico; leitura para no `len` lógico como hoje.

## Result (`:p255`, 3 rodadas válidas)

- A4 mediana válida **0,6486** (0,6069 / 0,8786 / 0,6486) — dentro do
  corredor 0,6176–0,7175 das ondas `:p252`–`:p254`. Sem pay. Melhor
  rodada válida **0,8786** (era 0,7982) e melhor absoluto limpo da
  pedra **230,8k** (era 210,6k) — mas o rocks da mesma onda também
  subiu (r1 292,9k; canário r3 305,7k, o mais alto da série).
- A5 mínimo **0,5810** (0,5810 / 0,9605 / 0,9195). Sem pay.
- C2: fjall **não-colapsado pela 1ª vez** (339k / 290k / 272k) e pedra
  venceu os 3 pares (+7,1% / +17,8% / +16,6%) — mas fjall 10–28%
  abaixo da única referência saudável (377243, `:p249`) e a pedra
  (316–363k) também abaixo dela. **Não declaro vitória.** Condição
  restante nomeada: rodada com fjall ≥ ~377k na qual a pedra vença.
- EG1 segue 62%. A engineering win fica: a reserva agora cobre a
  região efetivamente escrita (buraco esparso + deriva de
  `prealloc_to` eram bugs reais). Próximo dono nomeado do A4/A5: **o
  convoy da write lock** (`lock_wait` 18,83 µs vs ~2 µs de seção
  crítica com 4 solo writers; amplificação ~9× = cascata de futex) —
  RFC-0255.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** `preallocate_file` com offset + `preallocate_at` na
  trait + forwards + `reserve_space` ancorado na fronteira com teste
  de tamanho lógico inalterado e forward no io-uring — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Onda Linux `:p255`, 3 rodadas, sem phase stats, canário
  ≥165000, peer `sync:false`, A4/A5/C2 — status: `done` (impago;
  `findings/2026-09-23-rfc0254-a4-a5-c2-prealloc-frontier-linux/`)

### P2 — later / polish
- [x] **P2.1** Se o A4 seguir impago, o próximo dono nomeado é o
  refault/residência da janela sob pressão (populate da janela no
  slide ou mlock) e depois a `lock_wait` em si (convoy da write lock
  com 4 solo writers) — status: `done` (donos nomeados: convoy da
  write lock como próximo corte, RFC-0255; refault da janela fica
  atrás dele)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | âncora de pré-alocação na fronteira | done | RFC-0254 | 2026-09-23 |
| P1.1 | p1 | onda Linux A4/A5/C2 | done | :p255 impago | 2026-09-23 |
| P2.1 | p2 | refault da janela / lock_wait | done | convoy → RFC-0255 | 2026-09-23 |

## Acceptance Criteria
- **Tests:** posix — `preallocate` com offset além do EOF não cresce o
  tamanho lógico (KEEP_SIZE) e continua `Ok` em FS sem suporte;
  io-uring — `preallocate_at` repassado mantém o tamanho lógico; wal —
  appends com reserva cruzando chunks continuam lendo só até o fim
  lógico na recuperação (teste existente) e o tamanho lógico do
  segmento nunca passa do ponto de escrita.
- **Telemetry / Analytics:** nenhuma na onda pagante (sem
  `PEDRA_WRITE_PHASE_STATS`/`PEDRA_WRITE_SPIN`).
- **Documentation:** este RFC + TRAJETORIA v10 no land do resultado.
- **Screenshots:** backend-only.

## Out of scope
- Mudar o vlog para `preallocate_at` (o âncora atual do vlog é
  correto — não há `ftruncate` arrastando o `i_size` dele).
- Grow do `glue.kernel_fn_allowlist` (assinatura esticada no mesmo
  nome `preallocate_file`; método de trait com default).
- Heurística de trocar mmap↔pwrite por tamanho de DB.
