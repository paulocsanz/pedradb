# 2026-09-10 — RFC-0193 P1.1 — io_uring ordenado: veredito datado (não aterrizado)

**Veredito: fechado sem código nesta árvore** — o desenho continua sendo o
aprovado do 0189 P1.3 (SQEs encadeados por ticket via `IOSQE_IO_LINK`, um
submitter, fallback = pwrite do P0); nada foi construído sem poder medir.

## Por que não aterrizou

1. **Sem hospedeiro Linux**: io_uring não executa em Darwin; o guest
   `linux-gate-p149b` não resolve (ver `2026-09-10-rfc0193-p05-meter-blocked.md`).
   Aterrizar `unsafe` de io_uring no TCB `pedradb-posix` sem nenhuma máquina
   capaz de rodá-lo seria exatamente o "untested performance claim" que o
   RFC proíbe (risco listado no plano: "never as an untested performance
   claim").
2. **O prêmio é condicional ao meter do P0**: o P0 (pwrite off-lock) já move
   o `write()` para fora do `wal.lock()`. io_uring só paga ALÉM disso se a
   syscall por grupo ainda for dona após o re-split quieto — decidir isso
   antes do meter do P0 seria construir o degrau 2 sem medir o degrau 1.

## O que foi verificado (a perna executável do slice)

- **Cross-target check**: `cargo check -p pedradb-core --target
  x86_64-unknown-linux-musl` → exit 0 (`$S/xcheck-linux-musl.txt`). O seam
  `EnvFile::write_all_at_shared`/`try_clone_handle` do P0 (cfg unix, std
  `FileExt::write_all_at`) compila no alvo Linux — o ponto de extensão onde
  io_uring entraria (implementação alternativa de `EnvFile` no TCB posix)
  está provado tipicamente, sem `unsafe` novo em `pedradb-core`
  (`#![forbid(unsafe_code)]` mantido).
- **Fallback por desenho**: P1.1 é uma implementação de `write_all_at_shared`
  + drenagem; o P0 chama só pelo seam. Trocar pwrite→io_uring não muda o
  contrato do ticket nem os bytes do WAL.

## Reabrir (condições)

1. Gate Linux passa (guest resolve / caixa oficial + load quieto).
2. P0.5 meter mostra o pwrite off-lock KEEP e o re-split ainda nomeia
   `wr`/syscall como dono > ~0,4 µs/op.
3. Então: TCB `pedradb-posix`, cfg `target_os = "linux"`, `SAFETY.md`
   atualizado (lifetime do buffer até CQE, um submitter, IOSQE_IO_LINK
   por cadeia de ticket), fallback = pwrite P0, suíte recovery no Linux.
