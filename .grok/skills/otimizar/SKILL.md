---
name: otimizar
description: >
  Make Pedra faster than RocksDB default on every cartaz cell. The
  product of every fire is a measured number (qps, p50, avg_group,
  ratio, stall_us) — a SHA is not a land. Prove scale with diagnose
  (no 1B runtime). Use for otimizar, gargalo, ganhar, mapa, diagnose,
  escala, /otimizar. Not formal. Not overfit audit.
---

# /otimizar — o fire produz um número, não um SHA

## Grind pressure (one block, overwritten each fire)

- Last fire: worked (`ef0e937` P0.57 qps 7403→10270; STALL lead_write gone; remaining group_path 1.49s)
- Why: 64 MiB WAL F_PREALLOCATE on first commit (~1.5s Darwin)
- This fire MUST land: prealloc at Wal create + group_profile number beating max 1.49s / qps 10k
- Forbidden this fire: grouping knobs; JSON; Darwin as cartaz; SHA without `number:`
- Deeper: `Wal::create_on` `reserve_space`; test `rfc0180_wal_prealloc_at_create`

Peer: Rocks default `ROCKS_PARITY_SYNC=0`. G1 1c write-per-op ≠ win.
Fjall = absoluto. Darwin DIAG ≠ cartaz. Cartaz tables live in RFCs;
**this fire still pastes its own meter line.**

## Meter (first tool calls — skip = failed fire)

No engine diff and no RFC checkbox before this stdout exists. Paste it.

**Write cell** (overwrite_mc4 / grouping / stall) — async, peer class:

```bash
PEDRA_STALL_US=50000 cargo run -q --release -p rocksdb-parity-bench \
  --example group_profile -- 4 4000 /tmp/pedra-gp 100
```

Read `avg_group=` `qps=` `p50` `max` and any `STALL <phase> us=`.
G1 `put()` is not overwrite_mc4. Static `pedra diagnose write --clients 4`
`cut=grouping` is a lever **name**, not a number. WRITEPHASE OPS=32
seed-diluted `avg_group` is not a number.

**GET / probes** — gerador, no 1B runtime:

```bash
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 1000000000 --ram 68719476736
```

`cut=probe_path` / `class=as_is_walk` → cut probes, not disk.
`cut=indistinguishable` → do not cry walk-all.

**Cartaz ratio** — `rocks-parity-bench`, `ROCKS_PARITY_SYNC=0`, Linux 3-run.
Darwin = DIAG. Quiet-host overwrite_mc4 Rocks ≳260 kQPS. Collapsed Rocks ≠ win.

## Paid lever (do not re-pick)

| lever | paid when | next unpaid number |
|---|---|---|
| grouping 2–8 | timed `avg_group` ≥ `expected_group − 0.1` | `qps` / `p50_ns` / `stall_us` / max |
| wal on-lock | `STALL lead_write` / `lone_wal` ≪ previous max or gone | qps / p50 |
| fd_ceiling 1c G1 | always (not a win) | other cells |

Picking a paid lever again is a **failed fire**. After grouping paid,
`grouping_cap` / `wait_peer` / `last_peak` / `sibling_reentry` are paid.

## Valid land (exactly one; SHA is not the product)

Journal **must** contain a line from **this** fire's meter stdout:

```
number: avg_group=… qps=… p50_ns=… max=… stall_us=… (before: …) DIAG|cartaz
```

Same line first in `## Prova`. Missing, copied from a previous fire, or
only `cut=grouping` = **failed**. Grind autopsy: no `number:` = **noop**.

Then exactly one of:

1. **Engine** — production fn + named test. The unpaid number moved, **or**
   a new `STALL <phase> us=` the previous meter did not name (qps/p50/max
   still re-measured). Policy hole only with before/after of that meter.
2. **New cartaz shape** — real use missing from `COMPARE_SHAPES`. Same
   xorshift/zipf (`YcsbRunner`, seed `0x5EED_0001`). Append, never delete.
   Must-win → also `BALANCE_SHAPES` + mapa **U**. Still a number if you
   claim the cell.
3. **Kernel physics that did not exist** — new lever token or
   `predict_*_bottleneck` class. One-shot. Still a number if a cell moved.

Anything else (eprint clone, JSON field, mapa-only, RFC tick, grouping
knob after grouping paid) = **failed fire**. Patch Rank so that class
cannot be picked again, then do (1) or (2) this turn.

Caixa / 100M / 1B **não** desculpa turno vazio.

## 0. Gerador (sem runtime 1B)

Não WARM 100M. Não "preciso da caixa para saber o lever".

GET composto (P2.35, spec Intel 4 GHz): `--cache happy|capacity|cold`.
WRITE estático nomeia o lever; o **número** é o Meter acima.

```bash
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 50000000 --ram 68719476736 --cache capacity
PEDRA_WRITE_PHASE_STATS=1 ROCKS_YCSB_OPS=2048 ROCKS_PARITY_ONLY=deps_cache_overwrite_mc4 \
  ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SYNC=0 ROCKS_PARITY_MC_FRESH=1 \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- --engine compat
```

Mapa vs RFC: RFC + meter ganham. Actualiza `references/mapa.md` no mesmo change.

## 1. Board

Lê `references/mapa.md`. **S** Linux 3-run — não reabrir. **W** perda com
lever — cortar o número não-pago. **U** perda sem diagnose — Meter
primeiro. **C** teto — documentar. **T** kernel não emite — só física nova.

## 2. Rank (unpaid number, not unpaid RFC checkbox)

1. **W/U** cujo número não-pago o Meter já nomeou → corta essa fase.
   Grouping unpaid (`avg_group` < expected−0.1) vence COMPARE.
   Grouping **paid** → stall/qps, never another grouping knob.
2. Policy hole num segundo path, with the same meter before/after.
3. Uso real em `docs/benchmarks.md` / dependents **não** em `COMPARE_SHAPES`
   → land (2) acima.
4. **C** só documenta.
5. Caixa 3-run: diz bake, cai para 1–3. Nunca para.
6. Nunca skiplist sem `despark=1` (mem/gap ≥15% **e** clients≥2).
   Nunca Darwin como cartaz. Nunca T-clone (`classify_*` / `diagnose.lever`
   numa impressora nova).

Rank vazio de verdade: **diz o bloco**. Não inventes linha T.

## 3. Novo bench (harness que já existe)

- `Cfg` + `YcsbRunner::new` + `xorshift`. Os dois peers, o mesmo seed.
- Append `COMPARE_SHAPES`. Nunca apagar shape para subir min_ratio.
- Must-win → `BALANCE_SHAPES` + mapa **U** no mesmo turno.
- Só o trait `Engine`. Sem crate novo (RFC-0182). Sem `db_bench` C++.
- Depois: Meter + corte até S ou C nomeado. "Adicionei e perdi" não é win.

## 4. Ganhar em todos

Cartaz = `BALANCE_SHAPES` + cada must-win que (3) adicionar. Win = Linux
3-run mediana ≥1.0 vs Rocks `sync=false`, quiet. Perdas nomeadas ficam.
Fjall absoluto. G1 1c write-per-op é teto fd.

Depois de um corte: `pedra diagnose balance` no conjunto. S→W recusa o PR.

## 5. Output

```markdown
## Prova (gerador, sem 1B/caixa)
- number: avg_group=… qps=… p50_ns=… max=… (before: …) DIAG|cartaz
- get/probes: predict_get_bottleneck keys=… class=…

## Corte (um)
- Cell / unpaid number / fn de produção / teste nomeado
- Porquê esta: …
- Não fazer: skiplist; G1 1c win; Fjall gate; 4º crate; T-clone; paid lever

## Mapa
- linhas que mudaram de classe (pointers)
```

No `number:` line = failed fire. SHA in `## Corte` without that line = failed.

## Forbidden

- SHA + named test without journal `number:` from this fire's meter.
- N RFC P0s as "progress". Hours of grouping without qps/p50 is the failure.
- Paid lever again (grouping knobs after `avg_group` paid).
- `balance_admits=0` / single-shape 0180.
- Win vs `sync=true` ou G1-off.
- WARM 100M em 4 GiB. `PEDRA_BULK_CHUNK_BYTES=4MB`.
- RFC-0175. Push `origin` na árvore interna.
- Parar porque "é caixa" sem ter corrido o Meter.
- Static `cut=grouping` or seed-diluted avg_group as the number.
- Darwin quoted as Linux win.
