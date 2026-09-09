---
name: caminho-sel4
description: >
  Pay PedraDB’s seL4-class path this turn: production kernel rustc links is
  the proof term; then the I/O *script* (order of kernel+Env, not the
  syscall); then compose glue callers (ConcurrentDb / commit_ops_with);
  then the four concurrency theorems; then shrink remaining data-fate ifs.
  Catalog-only single_artifact is not a land. Always implement.
  Research Verus / Iris / Aeneas / DST / fuzzing and persist.
  Triggers: formalize, verificação, seL4, caminho sel4, trampolim, guião,
  concorrência, deadlock, data race, extraia aeneas, va formal,
  verification next, /caminho-sel4, /verificacao-next. Not benches.
---

# Caminho seL4 (Pedra)

## Grind pressure (one block, overwritten each fire)

- Last fire: worked
- Why: fc0375b6 / bf57bacf FailingEnv disk inject; this fire RFC-0179 P1.2 reclaim plan
- This fire MUST land: disk_pressure_reclaim_plan (WAL recycle + vlog GC + SST); db.rs reclaim_disk_for_uptime matches
- Forbidden this fire: leftover is_empty wrap factory; Montanha; twin=kernel wrap; dump db.rs; rank-7 SA
- Deeper: named test disk_pressure_reclaim_plan_on_live_reclaim_is_not_ok; skip Montanha

This skill **lands one payable slice per Fire**. Under `/grind`, that is
not the end of the turn: after the commit, Fire again (tools, land).
The §4 output template is status between Fires, never the last action.
A board-only reply is a failure. Rank continues after D/C/B/A empty.
Catalog-only `single_artifact` (Verus already last-wins on the file) is
**skip / fall-through**, never a halt.

**Slogan forbidden:** “somos seL4”, “sem bugs”, “garantia total”, “TSan is a
proof”, “PCT d=2 = ∀π”, “100% do Pedra”, “trampolim já é assembly”,
“catalog SA = preço seL4”.
**Price:** the `.rs` rustc links is the Verus/Aeneas term. `db.rs` /
`concurrent.rs` stay trampoline (`glue.db_rs_extracted=false`) — **empty
them** of data-fate `if`s and of unpaid *order*, do not dump them.
`never_floor` stays `R-cpu R-rustc R-verus R-crc R-deps`. Disk/`fdatasync`
Ok is not media (RFC-0078). `∀π` stays `lock_interleavings_admitted=false`.

Iterator / ConcurrentDb / deadlock / script order are **payable** (index
`while`, named fns those files already call, wait-for cycle, plan fn).
Refuse only a **measured** Charon/Aeneas/`lake` failure, named in
`formal/aeneas/EXTRACT.md`.

Pins: Charon `0.1.232` / Aeneas `daa85d7` / Lean `4.31.0`. Re-pin only if it
**widens** the translated set without `sorry`. Official Rocks peer stays
`ROCKS_PARITY_SYNC=0` (unrelated).

Recipes: `references/script.md` (order), `references/concurrency.md` (four
theorems), `references/aeneas.md` (extract/compose). Map:
`findings/2026-09-07-concurrency-proof-map/`.

## 0. Research (every invocation, bounded)

Search the last ~12 months (papers + practitioner blogs) for **one** of:
Verus / VerusSync / VerusBelt, Iris/RustBelt, Aeneas/Charon, DST /
deterministic concurrency, storage fuzzing (Walrus, CrashMonkey-class),
rely/guarantee locks. If it applies to a live Pedra kernel **or** to an
unpaid script/caller, persist PDF/text under `findings/YYYY-MM-DD-<topic>/`
and **use it in this turn’s slice**. Skip a second search if the first
already names the slice. Primary source over abstracts.

## 1. Search (mandatory)

```bash
python3 .grok/skills/caminho-sel4/scripts/candidates.py
```

Then open the files it named (handler body + plant body, not grep). Also
compute, from catalog + `residuals.json` (do not `json.dump` the live catalog):

- script/compose glue the board marked unpaid (`commit_ops_with`,
  `validate_occ_batch`, `lone_commit`, `finish_group_off_lock`, …)
- `data_fate` with `twin` path ≠ `kernel` path (real single-artifact debt)
- enrolled Aeneas path whose catalog `entry`/`as_is` has no Lean `def` and is
  not named in `EXTRACT.md` (silent leftover)
- ConcurrentDb / `db.rs` call to a kernel without Lean `unfold` of a
  **caller** fn (callee-only unfold is not payment — `concurrency.md`)

If script vs RFC disagree, **code + catalog win**.

## 2. Rank (first non-empty wins — then implement it)

Bound: **one** pair / one edge / one kernel fn **per Fire** (grind
chains Fires; this bound is not end-of-turn).

1. **D then C** — live `if` or cartoon plant vs catalog kernel (inbound
   `handle_inbound` / `PeerMsg::`, not `entry(` after `LiveQueued::open`).
2. **B / A** — freeze holes (`as_is` / `dst_plant` / `status=absent`).
3. **Silent leftover** — catalog `entry` on an enrolled extract path with no
   Lean `def`: extract (`--start-from`) or name the measured refuse in
   `EXTRACT.md`. Iterator CFailure: Isolated method (index `while`, not
   `impl Iterator`); patch generated Kernel in `scripts/aeneas_*.sh --required`.
4. **Script** — I/O/commit/group **order** still inline in `db.rs` /
   `concurrent.rs` (`references/script.md`). Extract a named total plan fn;
   production `match`es; theorem `need_sync ⇒ Sync before Apply/Ok` (AS-IS:
   Apply/Ok first). Pattern: `put_ok` / `WriteAckLedger`, but the **live**
   handler must call the plan. Not Env. Not dump. Board: `unpaid_script`.
5. **Compose glue callers** — Lean `unfold` of the **plan fn the handler
   calls** (`occ_batch_plan`, `wal_commit_plan`, …) **and** the callee, on
   a representative input **and** as-is/other branch (`aeneas.md`).
   `native_decide` of the plan without `unfold` is **not** compose.
   Callee-only (`group_validate` without `occ_batch_plan`) does not pay
   `validate_occ_batch`. Glue method names are never Lean defs — unfold
   the plan. Board: `unpaid_compose`. **Do not OR this with `calls_plan`.**
6. **Concurrency** — next unpaid of the four (`references/concurrency.md`):
   write-lock client protocol (`wal_rotate_decision` + `commit_inflight`);
   lost-update (`group_validate` / `occ_conflict` vs serialized);
   deadlock (`wait_for_deadlock` cycle, ConcurrentDb lock-order);
   N-way (`group_validate` of N `OccRead`s, not only 2);
   data-race tokens on RwLock **clients** after those (not `parking_lot`).
   `lock_interleavings_admitted = ok false` stays the scheduler theorem.
   Pull glue into a **named total fn**; do not extract `concurrent.rs` /
   `db.rs` whole.
7. **Single-artifact** — only if the catalog `entry` has **no** Lean `def`
   yet **and** the production file has no `cfg(verus_keep_ghost)`. Then
   put Verus on that file. If Lean already extracted the entry, or Verus
   already last-wins, **skip** — wrapping an already-extracted kernel
   (`ascii_lower is the file rustc links`) is not a slice. After extract
   freeze, this rank is empty. Do not fall through here while rank 5
   `unpaid_compose` is non-zero.
8. **atom→close** — one `data_fate` atom, production `entry`.
9. **F then E** — one clone group / one non-`data_fate` three-teeth pair
   whose `entry` has no Lean `def`. Already-extracted close pairs are
   **skip** (same SA-factory ban as rank 7).
10. **Scale / complexity** — only when 1–9 empty: Lean theorem over an
    already-enrolled `scale_kernel` / RFC-0176 model on a concrete N, or a
    DST/PCT plant that names the kernel (campaign ≠ ∀π).
11. **H** — L28/PCT campaign only if the user asked for that campaign.
12. Never dump `db.rs` / `concurrent.rs`. Never invent `l28_tcp_*_ok` gates
    that do not exist. Never flip `media_durable_admitted` or
    `forall_schedules_admitted` to true.
13. **Skip Montanha** until the user lifts this. No leftover wrap in
    `crates/pedradb-store/**` or `montanha-*` bins (fdb-bench, fdb-compare,
    perf-gate, tcp). `leftover_next` / journal `next:` must not name those
    paths. Engine leftover stays Pedra core / sim / world / raft / compat.
    Already-landed wraps stay.
14. **Skip leftover `is_empty`/`==0` wrap factory.** Wrapping a collection
    empty-check onto `batch_is_empty` is not a slice (same `n==0` body).
    Fall through. Next unpaid is a **data-fate plant**: DST/`FailingEnv`
    inject, script order, compose unfold, or a live `if` that decides
    WAL/OCC/fence/disk — never Montanha.

Tie-break: open RFC `- [ ] **P0`/`P1` on the same theme.

## 3. Land (same turn)

Follow `references/aeneas.md` for extract/compose; `references/script.md`
for a plan fn. Surgical HEAD-only commits on a dirty tree (posix SOURCE =
git HEAD if `lib.rs` is dirty). Mutant: wrong `glue.proof_depth.extract` in
a **copy** of residuals FAILs; restore from bak; never `json.dump` live
`catalog.json`.

Acceptance (all):

- a production `.rs` that rustc links **changed this turn**, or a new
  Lean/Verus theorem that `unfold`s a production caller **and** callee
  introduced or first used this turn
- named `cargo test` calls that **production** fn
- `scripts/lean_extracts.sh --required` exit 0 if Lean changed
- `--lint` freeze: extract count matches; `db_rs_extracted` false

**Not a land:** catalog/`residuals` `single_artifact` flag without the rustc
body change above; cfg/verus wrap on a kernel whose `entry` already has a
Lean `def` (SA factory; extract count unchanged); callee-only unfold billed
as a ConcurrentDb caller; `native_decide` of a plan without `unfold`;
board/rank/RFC checkbox only. If the first rank hit is that, fall through
and land the next unpaid item **this turn**.

## 4. Output (after the commit, not instead of it; not a stop)

Under `/grind` this section is **not** permission to stop. Write it
only if the next Fire's land tools already follow. Journal + pressure
+ grep is not that Fire.

```markdown
## Caminho seL4 — não acabou

Freeze: extract=… close=… atom=… model=… | single_artifact=…/data_fate=…
TCB: never_floor unchanged; db.rs trampoline; disk not media; ∀π refused.

### Landed (this turn)
- Slice: …
- Evidence (file:line + test name): …
- Theorem(s) / kernel / plant: …

### Not proved (this turn)
- disk/`fdatasync` media · ∀π · dump of db.rs/concurrent.rs · …

### Next on the path (one line)
- …

### Research incorporated (or none this turn)
- …
```
