# RFC-0193 P0.1 — captura `wal-before.bin` (gate de igualdade de bytes)

**Data:** 2026-09-10
**Comando** (pré-mudança, fonte em `main` + 0190/0189-P0.3 aterrissados, sem
nenhuma linha do 0193 ainda):

```
cargo run -q -p pedradb-core --example wal_capture -- <out-dir>
```

- Exemplo: `crates/pedradb-core/examples/wal_capture.rs` — single-thread,
  sequência fixa: 8 puts async 1c (pipeline), 5 puts G1 (`WriteOptions{sync:Some(true)}`),
  20 puts mesmo valor (interning v2), 1 delete async, 1 valor de 40 000 B
  (atravessa blocos de 32 KiB — First/Middle/Last), 1 put G1 de cauda,
  close + reopen (caminho append) + 4 puts + 1 delete.
- Caminhos reais: `ConcurrentDb::put`/`put_with`/`delete` (pipeline 1c e grupo
  verificado/G1), `Wal::append_on` no reopen.
- **44 055 bytes**, sha256 `831d94d6f7adeb2f8d6f33c80f51ece7c557633e79c0510473a42189c8a95c5d`.
- Duas execuções pré-mudança cmp-idênticas (determinístico).

## Gate

Pós-P0.4 (líder real off-lock), o MESMO comando produz `wal-after.bin`;
`cmp wal-before.bin wal-after.bin` deve ser idêntico byte a byte. A captura
cobria encode+append sequencial in-lock; o ticket `pwrite` tem de reproduzir
exatamente esses bytes (offsets dos tickets = offsets de append sequencial
por construção).
