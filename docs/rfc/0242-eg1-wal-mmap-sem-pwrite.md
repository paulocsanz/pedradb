# RFC-0242 — EG1: WAL de 1 op sem `pwrite` (memcpy MAP_SHARED)

**Status:** in-progress (corte landado; onda Linux ainda não mede este binário)
**Updated:** 2026-09-22
**ID:** 0242
**Parents:** [TRAJETORIA](../TRAJETORIA.md), [0241](0241-eg1-fim-do-statvfs-por-put.md)

## Contexto

A semente A8 de 100M chaves no gate box (`:p243`, binário com o cache de
sonda do 0241) fez **`syscw` ≈ 1e8** — um `pwrite` por chave, ~160 bytes.
O caminho committed de `IoUringFile::posix_pwrite` é `write_at` em todo
frame. Fjall não paga um syscall por insert. O A/B mmap×write(2) do 0240
foi medido num box lento (~100k qps contra 271k do protocolo) e não fecha
a questão no binário landed.

## P0

- [x] **P0.1** Frames ≤ 256 KiB em arquivo `*.log` vão por `MAP_SHARED`
  memcpy. A janela cresce de 64 MiB. SST e MANIFEST continuam `pwrite`
  (estender o MANIFEST quebra o CRC do CURRENT). `PEDRA_WAL_MMAP=0`
  volta ao `pwrite`. Sem `msync`: a página compartilhada é o page cache,
  a mesma classe do `pwrite` sem `fdatasync`.
  — status: `landado`
- [x] **P0.2** Teste `rfc0242_small_puts_skip_per_op_pwrite_and_reopen`:
  2000 puts no `ConcurrentDb` async, ≥ 1500 cópias mmap, valores vivos
  depois do reopen. Suíte `pedradb-io-uring` 24/24.
  — status: `verde (Darwin dev)`

## P1

- [ ] **P1.1** Onda Linux no próximo binário (não redeploy enquanto `:p243`
  estiver medindo o 0241): C2 `fjall_seq_1m` absoluto vs fjall, A4 e A5
  async, 3 rodadas, canary ≥ 165000, peer `sync: false`. Paga ⇒ fatia
  fecha; não paga ⇒ número datado.
  — status: `imagem pronta, não deployada` — `ghcr.io/paulocsanz/pedradb-linux-gate:p244`
  digest `sha256:d4369dcf…`, binário musl `a39331f2` sha256
  `6fdeba52…`. Deploy só depois de puxar os JSON da p243: redeploy
  recria `/data2`.

## Status

| slice | status | evidência |
|---|---|---|
| P0.1 mmap WAL ≤ 256 KiB | landado | `IoUringFile::posix_pwrite` |
| P0.2 teste nomeado | verde | `rfc0242_small_puts_skip_per_op_pwrite_and_reopen` |
| P1.1 onda Linux | bloqueada pela p243 | não redeployar o gate box agora |
