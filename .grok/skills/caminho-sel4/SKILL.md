---
name: caminho-sel4
description: >
  Pay PedraDB’s seL4-class path this turn: production kernel rustc links is
  the proof term; then the I/O *script* (order of kernel+Env, not the
  syscall); then compose glue callers (ConcurrentDb / commit_ops_with);
  then the four concurrency theorems; then shrink remaining data-fate ifs.
  Catalog-only single_artifact is not a land. Always implement: first
  unpaid rank 1–10, else one remaining data-fate `if` in the trampoline
  (Aeneas of rustc types the handler passes; cartoon remaining = delete
  the Verus stand-in — never mint a u64 twin, never skip it).
  Research Verus / Iris / Aeneas / DST / fuzzing and persist.
  Triggers: formalize, verificação, seL4, caminho sel4, trampolim, guião,
  concorrência, deadlock, data race, extraia aeneas, va formal,
  verification next, /caminho-sel4, /verificacao-next. Not benches.
---

# Caminho seL4 (Pedra)

## Grind pressure (one block, overwritten each fire)

- Last fire: worked (`f1bb75ae` fold_kernel.rs rustc `&[u8]` is the term)
- Why: leftover_next cartoon remaining `world_kernel.rs` (cfg split; toy Sample/u8 ≠ rustc TrajectorySample)
- This fire MUST land: delete `world_kernel.rs` verus stand-in + cfg split; rustc TrajectorySample body stays; Aeneas of that body
- Forbidden this fire: mint u64/`Seq` twin; leftover_next store/Montanha; leftover is_empty wrap; compact_refuse spray; git checkout of db.rs/concurrent.rs; skip cartoon as "not a land"
- Deeper: Search leftover_next_first world_trajectory world_kernel.rs; delete stand-in; named cargo; aeneas_world.sh --required; lake World

This skill **lands one payable slice per Fire**. Rank 1–10 unpaid first.
When those boards are 0, Search prints `leftover_next`: cartoon remaining
if `cartoon_twin>0`, else trampoline remaining — **not a halt**. Do not
invent a `compact_refuse` / `batch_is_empty` wrap. Do not replace
`leftover_next` with a spray-queue fn name. Catalog-only `single_artifact`
is skip **only if** Lean has `def` of the rustc entry **and the file is
not a cartoon**. `twin==kernel` + cfg split is unpaid. Empty 4–10 is
**not** a stop.

Pay the seL4-class path this fire (empty the trampoline of data-fate
`if`s). `never_floor` / disk-not-media / `∀π` refused stay TCB — not
permission to halt. Wrap/spray is not payment. Empty 4–10 is trampoline
remaining only after cartoon remaining is 0.
**Price:** the fn rustc links, with the types the handler passes
(`key::ValueType`, `&[u8]`, `Bound`), is the term. Pay with Aeneas
extract of **that** body. Verus last-wins only when it type-checks
**those types** (`macro_rules!` over types both compilers share: bool,
u64 counters, enums defined in the same file — RFC-0171 `idle_body!`).
A `verus!` toy enum / `u64` / `Seq<u8>` stand-in while rustc has bytes
is a **model twin**, not last-wins. `skip_verus_last_wins` (any
`verus_keep_ghost` = paid) is invalid. `_body!` over different types is
the same lie. Cartoon is unpaid. **Payment:** delete the stand-in and
the `cfg(verus_keep_ghost)` split; rustc body stays; Aeneas of that
body. Minting a twin is not a land. Skipping as "not a land" is not a
land. Bound-helper does not pay cartoon debt.
`db.rs` /
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

Then open the files the **UNPAID** rows named (handler body + plant body,
not grep). When leftover_next says cartoon remaining, open that kernel
file and **delete** the `verus!` stand-in (do not mint a replacement).
When leftover_next says trampoline remaining, open the handler that
still has a data-fate `if` without a kernel call, or the rustc body
whose extract has no Lean `unfold`. Also
compute, from catalog + `residuals.json` (do not `json.dump` the live
catalog):

- script/compose glue the board marked unpaid (`commit_ops_with`,
  `validate_occ_batch`, `lone_commit`, `finish_group_off_lock`, …)
- catalog kernel with a `verus!` stand-in or cfg split (unpaid cartoon even
  if `twin==kernel` / `single_artifact` / not `data_fate`)
- `data_fate` with `twin` path ≠ `kernel` path (real single-artifact debt)
- enrolled Aeneas path whose catalog `entry`/`as_is` has no Lean `def` and is
  not named in `EXTRACT.md` (silent leftover)
- ConcurrentDb / `db.rs` call to a kernel without Lean `unfold` of a
  **caller** fn (callee-only unfold is not payment — `concurrency.md`)

If script vs RFC disagree, **code + catalog win**.

`leftover_next` is **computed**. Unpaid 4–10 → those boards. Else if
`cartoon_twin>0` → cartoon remaining (named kernel file: delete the
stand-in). Search scans **every catalog kernel**, not only `data_fate`.
`twin==kernel` / `single_artifact: true` with a `verus!` stand-in or
cfg split is unpaid cartoon (rustc `&[u8]` vs Verus `Seq<u8>` /
clone_bytes / u64 flattened args is the same lie). Else trampoline
remaining. Never a production fn name, never FACTORY_BAN halt, never
mint, never skip cartoon as "not a land".

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
7. **Single-artifact** — Lean extract of the **rustc** body (same types
   the handler passes). Verus only if it type-checks **those types**.
   Cartoon (toy enum / `u64` / `Seq<u8>` ≠ rustc) is unpaid. **Land =
   delete the stand-in and the cfg split**; Aeneas of the rustc body.
   Do not mint. Do not skip as "not a land". Do not `_body!` over
   different types. Do not mint `#[cfg(not(verus_keep_ghost))]`.
   `skip_verus_last_wins` is invalid. If Lean already has `def` of the
   rustc entry **and the file is not a cartoon**, skip (SA wrap). If
   4–10 empty, leftover_next is cartoon remaining then trampoline
   remaining — **not a halt**. Do not fall through here while rank 5
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
    `crates/pedradb-store/**` or `montanha-*` bins. `leftover_next` must
    not name those paths.
14. **Skip leftover wrap factory.** Same `n==0` / `is_empty` body is
    not a slice. Same for a kernel whose body is `a || b`, `a && b`,
    `a == b`, `a != b`, or identity on a bool the handler already
    computed (`inline_needs_escape`, `vlog_closed`, `s >= e` renamed).
    Same for minting a Verus cartoon (u64 / toy enum / `Seq<u8>` ≠ rustc
    types) or billing one as last-wins. Fall through is **not** “wrap
    the next operator” and **not** skip cartoon remaining.
15. **DiskPressure is write admission.** Only a **new user/ops write**
    that would append WAL or write SST/dest (`put` / `delete` /
    `apply_batch` / `flush` / `compact*` / PITR dest / replica append).
    **Never** `close` / `Drop` (`close` takes `self` — Err drops the
    handle). **Never** post-commit finish (`compact_vlog_promote`,
    `rotate_wal_now` after SST durable). **Never** best-effort auto-flush
    (F18). Slapping `compact_refuse` on the next fn is the wrap factory.
16. **Wrap factory is not a slice** (`is_empty`/`compact_refuse` spray /
    `||`/`==` identity kernel / minting a Verus cartoon / DiskPressure on
    `close`/promote/rotate-after-SST/auto-flush). When unpaid 4–10 is 0,
    leftover_next is cartoon remaining (delete the stand-in) then
    trampoline remaining. Do not wrap. Do not halt. Do not mint. Do not
    skip cartoon. RFC P1.3 telemetry is not data-fate. P2.1 fence blast
    is deferred. Inventing a disk `if` to have a SHA is shallow — revert.

Tie-break: open RFC `- [ ] **P0`/`P1` on the same theme, only if it is
write-admission or unpaid compose/script — not a new `compact_refuse`
site.

## 3. Land (same turn)

Follow `references/aeneas.md` for extract/compose; `references/script.md`
for a plan fn. Surgical HEAD-only commits on a dirty tree (posix SOURCE =
git HEAD if `lib.rs` is dirty). Mutant: wrong `glue.proof_depth.extract` in
a **copy** of residuals FAILs; restore from bak; never `json.dump` live
`catalog.json`.

Acceptance (all):

- a production `.rs` that rustc links **changed this turn**, or a new
  Lean theorem that `unfold`s a production caller **and** callee
  introduced or first used this turn (Verus theorem only if it
  type-checks those rustc types — same types, not a u64/`Seq<u8>` stand-in).
  Deleting a cartoon stand-in from that `.rs` **is** this change.
- named `cargo test` calls that **production** fn
- `scripts/lean_extracts.sh --required` exit 0 if Lean changed
- `--lint` freeze: extract count matches; `db_rs_extracted` false

**Not a land:** catalog/`residuals` `single_artifact` flag without the rustc
body change above; keeping a Verus `verus!` stand-in whose signature ≠
rustc (`u64`/`Seq<u8>` vs `&[u8]`); `cfg(verus_keep_ghost)` wrap billed
as last-wins; `_body!` over different types; skipping cartoon remaining
as "not a land"; Bound-helper billed as cartoon payment; cfg/verus wrap
on a kernel whose `entry` already has a Lean `def` **and is not a
cartoon**; callee-only unfold billed as a ConcurrentDb caller;
`native_decide` of a plan without `unfold`; board/rank/RFC checkbox only;
wrapping `is_empty`/`==0` onto `batch_is_empty`; wrapping a live `||` /
`==` / `!=` / identity-bool into a kernel whose spec is that operator;
slapping `compact_refuse`
(or any paid kernel) onto the next production fn `leftover_next` named;
`DiskPressure` on `close` / `Drop` / promote / rotate-after-SST /
auto-flush; replacing `leftover_next` with a function name;
`include_str` of a kernel the handler already calls; minting a Verus
u64/toy-enum twin. If the first rank hit is mint/wrap/halt, fall through
to cartoon remaining (delete the stand-in) then trampoline remaining.

## 4. Output (after the commit, not instead of it)

Under `/grind` this section is **not** permission to stop. Write it
only if the next Fire's land tools already follow. Empty 4–10 is
cartoon remaining then trampoline remaining, not a stop. Minting a
cartoon is not a land; deleting one is.

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
