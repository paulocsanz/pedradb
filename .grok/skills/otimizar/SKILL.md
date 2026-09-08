---
name: otimizar
description: >
  Make Pedra faster than RocksDB default on every Linux cartaz cell.
  The product of a fire is Pedra vs Rocks `sync=false` on a named-loss
  cell (or a new dependent shape with that ratio) — never a Pedra-only
  Darwin micro. Use for otimizar, gargalo, ganhar, mapa, diagnose,
  escala, /otimizar. Not formal. Not overfit audit.
---

# /otimizar — o fire produz um ratio vs Rocks, não um SHA Darwin

## Grind pressure (one block, overwritten each fire)

- Last fire: worked (yugabyte_docdb_rmw 0.732 DIAG; rockset_hybrid 0.123→0.811 via WriteBatch; ycsb_b_mc4 0.060 DIAG)
- Why: Linux overwrite_mc4 0.557× still unpaid; RwLock spin/fair already exists (RFC-0045) — do not re-land
- This fire MUST land: engine cut on overwrite_mc4 or ycsb_b_mc4 get_path — not another lock spin
- Forbidden this fire: PEDRA_WRITE_SPIN/FAIR duplicate; group_profile; Darwin as Linux
- Deeper: ycsb_b_mc4 get_path; overwrite lock_wait hold time (not acquire spin)

Peer: Rocks default `ROCKS_PARITY_SYNC=0`. G1 1c write-per-op ≠ win.
Fjall = absoluto, never a ratio win. **Linux 3-run = cartaz. Darwin vs
Rocks = DIAG of that cell. Pedra-only `group_profile` is not a number
while any Linux <1× cell is unpaid.**

When the user asks "progress", "benchmarks", "<1×": answer the **Linux
vs Rocks** table first. Darwin group_profile is not the answer.

## Meter (first tool calls — skip = failed fire)

The unpaid number is `compat_over_rocksdb` on the rank-1 cell. No engine
diff before this stdout exists.

**Write / YCSB / dependents cell** (the Linux <1× row):

```bash
export ROCKS_PARITY_SYNC=0 ROCKS_PARITY_MC_FRESH=1 ROCKS_PARITY_CLIENTS=4
export ROCKS_PARITY_ONLY=deps_cache_overwrite_mc4   # rank-1 cell
OUT=/tmp/pedra-vs-rocks
for eng in compat rocksdb; do
  feat=; [ "$eng" = rocksdb ] && feat="--features real"
  cargo run -q --release -p rocksdb-parity-bench $feat \
    --bin rocks-parity-bench -- "$OUT/$eng" "$eng"
done
ROCKS_PARITY_PEER="$OUT/rocksdb/rocks_parity_bench.json" \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    "$OUT/compat/rocks_parity_bench.json" "$OUT/compare"
```

Paste `compat_over_rocksdb=` and both QPS. Quiet-host overwrite_mc4 Rocks
≳260 kQPS. Collapsed Rocks ≠ win. `sync: true` in the peer JSON → refuse.

`group_profile` / `PEDRA_STALL_US` / static `cut=grouping` / seed-diluted
WRITEPHASE = **lever probe**, not the fire's number.

**GET / probes** — gerador, no 1B runtime:

```bash
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 1000000000 --ram 68719476736
```

Linux 3-run is cartaz. Darwin same-boot vs Rocks is DIAG. Caixa blocked
→ still run the Meter above (Darwin DIAG of the Linux cell), then cut.

## Linux <1× (same-class) — rank lives here

Unpaid until Linux 3-run ≥1.0 quiet, or a named ceiling (C):

| cell | Linux cartaz | notes |
|---|---|---|
| overwrite_mc4 | **0.557×** 3/3 | rank 1. Isolated Darwin mediana 1.002 named loss 0.816. Caixa not re-run after 0180 |
| ycsb_f_mc4 | mediana 1.47, run2 **0.766×** | 3/3 quiet still unpaid |
| apply_mc4 same-class | not in floor 15/15 | G1 2.79× is another column. Darwin DIAG 0.48× |
| prefix 100M @ 4 GiB | **0.70×** | caixa bounded-cache. Big-guest 100M is 1.05× |
| kvrocks_set_mc50 | **0.37×** | **C** Adaptive-off n≥16 — document, do not "win" |
| ycsb_b_mc4 | no Linux 3-run | in BALANCE_SHAPES — measure vs Rocks |

`group_profile` avg_group≈4 does **not** pay overwrite_mc4.

## Paid lever (do not re-pick)

A lever is paid when **that cell's vs-Rocks ratio** no longer names it,
or when it is a named ceiling. `avg_group≈4` on Darwin does not pay a
Linux 0.557× row. After grouping is a Darwin DIAG fact, the unpaid
number is still `compat_over_rocksdb` on that shape.

## Valid land (exactly one)

Journal **must** contain, from **this** fire's Meter stdout:

```
number: ratio=… pedra_qps=… rocks_qps=… shape=… (DIAG|cartaz)
```

No `ratio=` vs Rocks = **failed**. `avg_group` / `qps` / `stall_us` from
`group_profile` = **failed** while a Linux <1× cell is unpaid.
Copied number = failed. Grind: that journal line missing or Pedra-only = **noop**.

Then exactly one of:

1. **Engine** — production fn + named test. The unpaid **ratio** moved
   (DIAG or cartaz), or a policy hole on the rank-1 cell with before/after
   of that same vs-Rocks meter.
2. **New cartaz shape** — real use from `docs/rocksdb-dependents-benchmarks.md`
   (or Fjall/Pebble-class workload) **missing** from `COMPARE_SHAPES`.
   Same `YcsbRunner` / xorshift seed `0x5EED_0001`. Append, never delete.
   Run vs Rocks (`SYNC=0`) this fire; paste `ratio=`. Must-win → also
   `BALANCE_SHAPES` + mapa **U**.
3. **Kernel physics that did not exist** — new `predict_*` class. One-shot.
   Still a vs-Rocks number if it claims a cell moved.

Anything else (group_profile P0, eprint clone, JSON field, mapa-only,
RFC tick) = **failed fire**.

## 0. Gerador (sem 1B)

Não WARM 100M. Não "preciso da caixa para saber o lever".
WRITE estático nomeia o lever; o **número** é o Meter vs Rocks.

## 1. Board

Lê `references/mapa.md`. **S** Linux 3-run — não reabrir. **W** perda
com lever — cortar o ratio. **U** perda sem diagnose — Meter vs Rocks
primeiro. **C** teto — documentar. **T** kernel não emite — só física nova.

## 2. Rank (Linux <1× first)

1. Linux **same-class <1×** in the table above (overwrite_mc4, then
   ycsb_f_mc4 3/3, then apply_mc4 same-class). Meter vs Rocks, then cut.
2. `BALANCE_SHAPES` row with no Linux number (today: ycsb_b_mc4) → measure.
3. Real dependent / Fjall-class use **not** in `COMPARE_SHAPES` → land (2).
   Source: `docs/rocksdb-dependents-benchmarks.md` (internal) / RFC-0043.
   Do not re-add Kvrocks/MyRocks/Surreal/Nebula/Ceph already in COMPARE.
4. **C** só documenta (kvrocks_set_mc50, G1 1c fd-ceiling).
5. Caixa 3-run: diz bake, **cai para 1–3** (Darwin vs Rocks DIAG or new
   shape). Never stop. Never fall through into `group_profile`.
6. Nunca skiplist sem `despark=1`. Nunca Darwin como cartaz. Nunca T-clone.

Rank vazio de verdade: **diz o bloco**. Não inventes linha T.

## 3. Novo bench (harness que já existe)

- `Cfg` + `YcsbRunner::new` + `xorshift`. Pedra **e** Rocks, mesmo seed.
- Append `COMPARE_SHAPES`. Nunca apagar shape para subir min_ratio.
- Must-win → `BALANCE_SHAPES` + mapa **U** no mesmo turno.
- Só o trait `Engine`. Sem crate novo (RFC-0182). Sem `db_bench` C++.
- Fjall: `--features fjall`, QPS absoluto, never `compat_over_rocksdb` as win.
- "Adicionei e perdi" não é win — keep the named loss.

## 4. Ganhar em todos

Cartaz = `BALANCE_SHAPES` + cada must-win que (3) adicionar. Win = Linux
3-run mediana ≥1.0 vs Rocks `sync=false`, quiet. Perdas nomeadas ficam.

## 5. Output

```markdown
## Prova
- number: ratio=… pedra_qps=… rocks_qps=… shape=… DIAG|cartaz
- Linux <1× still unpaid: …

## Corte (um)
- Cell / unpaid ratio / fn de produção / teste nomeado
- Não fazer: group_profile as product; G1 1c win; Fjall gate; 4º crate

## Mapa
- linhas que mudaram de classe
```

## Forbidden

- Journal number from `group_profile` / stall_us / Pedra-only qps while a
  Linux <1× cell is unpaid — **the hours-of-irrelevant-P0s failure**.
- Answering "progress / benchmarks / <1×" with Darwin DIAG first.
- SHA + named test without `ratio=` vs Rocks.
- Darwin quoted as Linux win. Collapsed Rocks as win.
- `balance_admits=0` / single-shape 0180.
- Win vs `sync=true` ou G1-off.
- WARM 100M em 4 GiB. `PEDRA_BULK_CHUNK_BYTES=4MB`.
- RFC-0175. Push `origin` na árvore interna.
- Parar porque "é caixa" sem Meter vs Rocks (Darwin DIAG of that cell)
  or a new dependent shape.
