---
name: verificacao-next
description: >
  Rank the next PedraDB *formal verification* slice. Searches catalog, residuals,
  RFC open checkboxes, live store/raft handlers, and DST plants (inbound vs
  pure-fn cartoon) before picking. Use for "próximo passo da verificação",
  "e agora formal", "acabou a garantia", "sem bugs?", "verification next",
  /verificacao-next. Not benches.
---

# Next verification slice (Pedra formal)

Rank **formal** work. Not benches. Not a proof the system is bug-free.

**Forbidden:** “acabou”, “perfeito”, “sem bugs”, “garantia total”, seL4.
Claim is only: kernel K ⊨ spec S relative to axioms A.

## Implement the winner (same turn — mandatory)

After the board + rank, **implement** the first non-empty ranked class
(D → C → B → A → atom→close → F → E) in this turn. Report-only turns are a
failure of this skill: the deliverable is landed code (kernel/guard/twin/wire),
not a description of it.

Bound the slice so it lands in one turn:

- **D/C:** one kernel + handler + inbound plant.
- **B/A:** one freeze hole / one twin.
- **atom→close (RFC-0170):** ONE `data_fate` atom pair per turn (never 26).
  Board line `atom_to_close <id> <entry>`. Close the production `entry`, not
  the inner atom.
- **F:** ONE clone group per turn (never all 7): expand the group's fns
  (twin per copy, agreement guard, or drift check), wire catalog, named test.
- **E:** one pair per turn (never 42).

Tie-break: an already-written RFC `- [ ] **P0/P1` naming a slice picks that
slice over an equally-ranked one (do not invent a new RFC to gate the work —
RFC naming is a tie-break, not a gate).

Acceptance: catalog wired, `--lint` names the pair, named `cargo test` green,
RFC checkbox `done` when one exists. Still print the board (winner may
already be `done`).

**Report-only (never implement): H / G / I.** Never extract `db.rs`.

## 1. Search (mandatory, this turn — do not skip)

```bash
python3 .grok/skills/verificacao-next/scripts/candidates.py
```

The same script also prints the capacity-per-residual table (RFC-0157 P2.2:
guard / checked twin / campaign depth / REAL anchor for all 28 residuals,
every reference verified against the repo; broken reference exits 1).

The script **searches** (it is not a memory dump): RFC `- [ ] P*`, catalog
holes, whether `store/src/lib.rs` mentions each protocol fn, and each
`data_fate` plant body (`handle_inbound` / `PeerMsg::` vs only `entry(`).

Then **open the files it named** (grep is not a substitute for the handler):

1. Residuals `never_floor` + `glue.db_rs_extracted`
2. The RFC line with `- [ ] **P1` / `P2` it printed
3. Catalog pair of the leading D/C id
4. Live handler around that pair (`on_request_vote` / `on_append_entries` / …)
5. The plant **test function body**, not just the file

If script vs RFC disagree, **code + catalog win**.

## 2. Board (name the inhabitant or write `none`)

| Class | Search found | Next slice if picked |
|-------|----------------|----------------------|
| **A** | `status=absent` | twin or recorded gap |
| **B** | `data_fate` missing `as_is` / `dst_plant` | 0151 freeze hole |
| **C** | store mentions entry without `live_callers`, or live_callers drift | freeze the live call |
| **D** | protocol bit ≠ catalog fn, or plant is `pure_fn` cartoon | wire fn + inbound plant |
| **E** | non-`data_fate` three-teeth 0/N | **one** pair, not 42 |
| **atom→close** | `twin_kind=atom` and `data_fate` | RFC-0170: prove the production `entry` |
| **F** | clones list | expand fns if tokens can drift |
| **G** | `never_floor` / extract | refused — not a next proof |
| **H** | L28 / PCT / lock ∀ | campaign, not theorem |
| **I** | 0149 / crates.io | not verification |

Cartoon plant = `dst_plant` test calls `entry(` after `LiveQueued::open`
(or `l28_campaign`) but never `handle_inbound` / `PeerMsg::`. That is class
**D** (or C) even if `--lint` is green. Core Db flush/scan plants are live,
not cartoon.

## 3. Rank (first non-empty wins)

1. **D** then **C** — live `if` or cartoon plant vs catalog kernel.
2. **B** / **A** — freeze holes.
3. **atom→close** — RFC-0170: one data_fate atom, production entry.
4. **F** — clone drift.
5. **E** — one non-`data_fate` pair.
6. **H** — only if the user asked for that campaign.
7. Never **G** or **I**.

Tie-break: already-written RFC P0/P1 beats inventing a new RFC.

## 4. Output

```markdown
## Verificação: não acabou

O que *não* é verdade: garantia total / sem bugs / seL4.
O que *é* o freeze hoje: <numbers from the script>.

### Próximo passo (um)
- **Slice:** …
- **Evidência (ficheiro:linha):** …
- **Porquê esta e não as outras:** …
- **Aceite:** teste nomeado + `--lint` naming the pair id
- **Não fazer:** extract `db.rs`; World seeds; benches

### Alternativas (rankeadas)
1. …
2. …
3. …

### Recusado / campanha
- never_floor: …
- campaign: …
```
