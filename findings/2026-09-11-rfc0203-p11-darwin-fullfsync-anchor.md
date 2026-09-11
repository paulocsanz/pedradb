# RFC-0203 P1.1 — âncora darwin `F_FULLFSYNC` vs `fdatasync` medida e datada

**Date:** 2026-09-11 · **RFC:** docs/rfc/0203-escada-viva-cotas-maquina-ancoras-classe.md

## Método (re-executável, committed no repo)

`crates/pedradb-posix/examples/fullfsync_anchor.rs`:

```text
cargo run -q --release -p pedradb-posix --example fullfsync_anchor
```

Por sabor, N=200 barreiras cronometradas (warm-up de 1 chamada antes de
amostrar; cada barreira precedida de um write real de 64 bytes para que
haja sujo a drenar): `fdatasync` via `pedradb_posix::fdatasync_file`
(libSystem `fdatasync` — a classe G1, RFC-0036) e `F_FULLFSYNC` via std
`File::sync_all` (que NO darwin é `fcntl(F_FULLFSYNC)`). Imprime p50 e
spread p10..p90 por sabor, o multiplicador intra-host p50, e o `uptime`
loadavg NO MOMENTO da medição — o rótulo quiet/DIAG se decide DELE.

## Medições (duas execuções, 2026-09-11, host macos-aarch64)

Run 1 (03:15, load averages: 10,32 16,11 14,84):

```text
fdatasync      p50=17667ns  spread(p10..p90)=16459..20417ns
F_FULLFSYNC    p50=4002000ns  spread(p10..p90)=3841333..4138666ns
ratio F_FULLFSYNC/fdatasync (p50): 226.5x
```

Run 2 (03:16, load averages: 10,02 15,86 14,77):

```text
fdatasync      p50=19792ns  spread(p10..p90)=17916..27458ns
F_FULLFSYNC    p50=4077083ns  spread(p10..p90)=3819458..4544083ns
ratio F_FULLFSYNC/fdatasync (p50): 206.0x
```

Duas execuções independentes concordam na classe: `fdatasync` ~18–20 µs,
`F_FULLFSYNC` ~4,0–4,1 ms, multiplicador intra-host ~206–227×. Os ~4 ms
do `F_FULLFSYNC` batem com o que o código já documentava
("~5 ms here" em `pedradb-posix/src/lib.rs`; "~4 ms" em `env.rs`).

## Rótulo honesto: DIAG

Loadavg 10–16 durante ambas as execuções (sessões paralelas nesta
máquina): a caixa NÃO está quiet. O valor registrado como âncora darwin
usa o run 1 (p50 `fdatasync`=17667ns, `F_FULLFSYNC`=4002000ns) com
rótulo **DIAG** — nunca quiet — até uma re-execução em máquina de fato
ociosa. O multiplicador de CLASSE (duas ordens de magnitude) é estável
entre os runs; o ns absoluto não é teorema nem pretende.

## Fronteira (0187 intocada)

- **Nunca teorema de ns, nunca em Lean**: os números vivem aqui e na
  tabela `scripts/ratchet/host_anchors.tsv` (P1.2), como âncora datada
  com fonte. Persistência física (TCG power-cut, perda de cache do
  drive) segue experimento — RFC-0187.
- **Multiplicadores count são class-independent**: os teoremas de
  contagem (`WorkIo.lean` — `wal_commit_plan_at_most_one_fdatasync`)
  contam CONSTRUTORES na álgebra `Work.io` (≤1 `fdatasync` por plano
  confirmado), válidos em qualquer classe de host; a âncora ns só
  preenche o custo físico por classe quando alguém pergunta.
- **Sem re-bench do Rocks**: o crates.io `rust-rocksdb` continua sem
  definir `HAVE_FULLFSYNC` (contexto: `findings/2026-08-27-upstream-fullfsync/`)
  — o peer no CMake só pede a classe `F_FULLFSYNC` com `-DROCKSDB_BUILD...FSYNC`;
  nada aqui muda a régua de paridade.

## Consumo

P1.2 registra a linha darwin em `scripts/ratchet/host_anchors.tsv`
citando este finding como fonte (mesma data, mesmo host, rótulo DIAG).
