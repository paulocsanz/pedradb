---
name: otimizar
description: >
  Rank and cut the next PedraDB *performance* slice from telemetry, not
  folklore. Always starts with WRITEPHASE / pedra diagnose / RFC-0176
  scale clock, then a living strong/weak map, then one surgical lever.
  Use for "otimizar", "gargalo", "próxima fase", "onde estamos fracos",
  "diagnose", "previsão de escala", "mapa de performance", "faça pedra
  ganhar", or /otimizar. Not formal verification (that's /verificacao-next).
  Not overfit audit (that's /audit-overfit).
---

# /otimizar — corte cirúrgico a partir da telemetria

Peer: RocksDB default `ROCKS_PARITY_SYNC=0`. G1 1c write-per-op não é win.
Fjall = absoluto, nunca gate. Darwin DIAG ≠ cartaz Linux. Não inventar
4º harness (RFC-0182). Não skiplist no escuro (RFC-0055/0183).

**Balance (lei):** nunca otimizar uma célula. O conjunto obrigatório é
`BALANCE_SHAPES` em `bench_gap_kernel` (hoje: overwrite_mc4, ycsb_a_mc4,
ycsb_f_mc4, apply_mc4, 1c overwrite). Novo caso de uso → **adicionar**
ao array + linha U no mapa, no mesmo turno. Um corte de engine só
passa se `balance_admits` (named-loss Linux **ou** ≥2 células cartaz).
DIAG sozinho `admits=0`. Depois do corte, voltar a diagnosticar o
conjunto — regressão S→W é recusa, não “a célula-alvo ganhou”.

**Números vivem nos RFCs / findings.** Este skill aponta; não duplica tabelas.

## Implement the winner (same turn)

After the board, **do the first unblocked cut** this turn:

- **Tool gap** (diagnose não classifica o shape / falta lever): kernel + teste + RFC-0184 checkbox.
- **Engine gap** with a named lever, `balance_admits=1`, and a local test: one lever. Then diagnose **all** `BALANCE_SHAPES`.
- **Caixa / 3-run Linux:** do **not** fake it on Darwin. Rank it, stop, say bake.

Report-only is a failure unless the winner is caixa.

Bound: one lever or one diagnose class. Then **expand** `references/mapa.md` and this skill if a new frontier appeared (new shape, new use-case, new lever token).

## 0. Search (mandatory — do not skip)

1. Read `references/mapa.md` (classes, not a second scoreboard).
2. RFC living tables: `docs/rfc/0176-*.md`, `0178-*.md`, `0180`–`0184` (pub: `~/software/pedradb-pub/docs/rfc/` if this tree lacks them).
3. Last `diagnose` line / WRITEPHASE / `PEDRA_COST_TRACE`. If missing for the candidate cell, **run or reconstruct** via:

```bash
# write (ns in one domain: per-op *or* per-commit)
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose write \
  --pedra-ns N --rocks-ns N --clients C --wal-ns N --mem-ns N --flush-ns N --lock-ns N

# get vs RFC-0176 clock
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys N --ram BYTES --measured-ns N

# mixed (ycsb A/F): --read-pct 50 → lever=get_path
# cost-trace vs P_best
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose probes \
  --per-get N --p-best N

# multi-shape gate (0182). --cell TOKEN[:diag|:named]
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose balance \
  --cut wal_encode_or_write \
  --cell get_path:diag --cell wal_encode_or_write:diag --cell flush_check:diag
```

Harness: `PEDRA_WRITE_PHASE_STATS=1` prints `diagnose <shape> dominant=… lever=…`.

4. Quiet-host: overwrite_mc4 Rocks ≳260 kQPS. Collapsed Rocks is not a Pedra win.

If mapa vs RFC disagree, **RFC + diagnose win**. Update the mapa in the same change.

## 1. Board (name the inhabitant or `none`)

Fill from search. Classes:

| Class | Meaning | Next if picked |
|-------|---------|----------------|
| **S** | strong on Linux official / 3-run | do not reopen as a loss |
| **W** | named loss, lever known | cut that lever |
| **U** | measured loss, **no** diagnose | measure + diagnose first |
| **C** | named ceiling (fd, Adaptive-off n≥16, 4 GiB bounded-cache) | do not "fix" into a win claim |
| **T** | tool cannot classify this shape | extend `bench_gap_kernel` + this skill |

## 2. Rank (first non-empty wins)

1. **U** on the **Linux** write cell that already lost 3/3 (today: `overwrite_mc4` caixa — 0178 P1.3 / 0184 P1.1). Diagnose before code.
2. **W** whose lever is local (flush_check, wal_encode_or_write, grouping 2–8). One lever.
3. **T** — mixed still `read_or_client` (no `--read-pct`); probes vs \(P_{\mathrm{best}}\) missing.
4. **C** only to document, never to hide.
5. Never treat Darwin same-boot mixed 0,5–0,7× as the Linux map.
6. Never 0055 skiplist unless `despark=1` (mem/gap ≥15% **and** clients≥2).

Tie-break: an open RFC `- [ ] **P0/P1` on that cell.

## 3. Output

```markdown
## Mapa (S / W / U / C / T)
- … (pointers, not a number dump)

## Próxima fase (uma)
- **Cell / host:** …
- **Diagnose:** dominant=… lever=… despark=…
- **Porquê esta e não as outras:** …
- **Corte:** … (código local / bake caixa / estender kernel)
- **Não fazer:** skiplist; G1 1c win; Fjall gate; 4º harness; corte com `balance_admits=0`

## Ferramenta / skill
- gap no diagnose? → kernel + teste + linha no mapa
- fronteira nova (shape/uso)? → linha U no mapa + harness **existente**
```

## 4. Expand (every successful turn)

When a new cell/use-case/lever appears, **edit** `references/mapa.md` in the same change (replace the row; one home). If diagnose grew a token, name it in the Board table above (one line). Do not paste RFC tables into the skill.

## Forbidden

- Single-shape engine PR (0180 P0.34–P0.38). `balance_admits=0`.
- "estamos a perder tudo" from Darwin DIAG.
- Win vs `sync=true` or G1-off bypass.
- WARM 100M on 4 GiB. `PEDRA_BULK_CHUNK_BYTES=4MB`.
- Implementing RFC-0175.
- Push `origin` on the internal tree.
