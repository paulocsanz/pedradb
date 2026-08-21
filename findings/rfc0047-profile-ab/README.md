# RFC-0047 — telemetry: perfil de retenção A/B + RecoveryReport + pin do bench

2026-08-21. Aceitação ("Telemetry"): A/B do perfil (F20 vs auto_reclaim no
mesmo workload de overwrite: disco final + scan), print do RecoveryReport, e
prova de que nenhuma coluna oficial muda sem o pin (`ROCKS_PARITY_RETENTION`).

Reproduzir: `cargo run -q --release -p rocksdb-compat --example rfc0047_profile_ab`

## A/B retenção (exemplo `rfc0047_profile_ab`, mesmo workload do teste
`auto_reclaim_default_matches_rocks_profile`)

20 rounds × 10 chaves × 1 KiB incompressível (xorshift), mesmas chaves
sobrescritas, `write_buffer_size=64 KiB`, flush por round. Live set = 10 KiB;
escrito = 200 KiB.

| perfil | disco final | full-scan reopen | chaves |
|---|---:|---:|---:|
| `auto_reclaim=true` (default drop-in, RFC-0047 P0.3) | 22 355 B | 51 µs | 10 |
| `auto_reclaim=false` (kernel F20) | 209 347 B | 63 µs | 10 |

F20 retém **9,4×** o disco do perfil Rocks (≈ tudo que foi escrito: 209 KB ≈
200 KB de payload + índice). Reclaim fica em ~2,2× o live set. O scan completo
no reopen é 51 vs 63 µs neste tamanho — o cliff de scan frio do F20 aparece em
tamanhos maiores; aqui o que diverge é disco.

## RecoveryReport (P0.2, default compat = PointInTime)

WAL com 8 registros, FlipCrc no físico #3, reopen no default do drop-in:

```
recovery_report: kind=crc
recovery_report: corrupt_offset=456 good_through_offset=456 discarded_bytes=760
recovery_report: prefix k02 visible=true suffix k03 discarded=true
```

Prefixo servido, sufixo descartado **e reportado** — nunca silencioso (G2).

## Pin do bench: nenhuma coluna oficial mudou com a virada de default

Prova mecânica (fonte, não opinião):

- **Pré-P0.3** (`83399bc^`): compat default `auto_reclaim: false`
  (lib.rs:186) e o bench só ligava reclaim com `ROCKS_PARITY_AUTO_RECLAIM=1`
  (default 0) — colunas oficiais mediam **F20**.
- **HEAD** (default `product`): pin força `reclaim=false` — colunas oficiais
  continuam medindo **F20**. Configuração medida idêntica dos dois lados da
  virada.
- Guarda: `ROCKS_PARITY_RETENTION=bogus` → **exit 2** (verificado ao vivo);
  conflito com a flag legada também recusa.

JSON antes/depois (mesmos params: deps suite, 4096/2000, payload 1000,
zipfian, MC4 — `bench-pin/head-deps-mc4/` vs `rfc0040-p11/run1/compat/`):

| shape | pré-P0.3 (17/08) | HEAD pin | razão |
|---|---:|---:|---:|
| deps_apply_batch | 1 719 | 3 286 | 1,91 |
| deps_mvcc_latest | 399 434 | 399 850 | 1,00 |
| deps_scan | 88 053 | 546 237 | 6,20 |
| deps_raftlog | 12 970 | 5 749 | 0,44 |
| deps_cache_overwrite | 22 870 | 25 873 | 1,13 |
| deps_apply_batch_mc4 | 2 459 | 10 842 | 4,41 |
| deps_raftlog_mc4 | 15 587 | 21 139 | 1,36 |

As razões **não** são retenção: entre 17/08 e hoje entraram commits de engine
(`e5c8c72` CountCache range-aware, `b149e3c` point-cache invalidate por chave)
que explicam scan/apply subirem; raftlog 0,44× é ruído de caixa (o README do
rfc0040-p11 já documenta 0,34–1,57 run-a-run). `deps_mvcc_latest_split` e
`deps_scan_probe` são shapes de sonda (sem `qps` por design, no JSON dos dois
lados). O ponto da comparação: mesmo schema, mesmas formas, mesma retenção
medida — a virada de default do compat não tocou a coluna oficial.

## Raw

- `stdout.txt` — saída do exemplo (A/B + RecoveryReport).
- `bench-pin/head-product/`, `bench-pin/rocks/` — ycsb 256/1000 nos dois pins
  (guarda de troca).
- `bench-pin/head-deps-mc4/` — reprodução dos params do rfc0040-p11 no HEAD.
