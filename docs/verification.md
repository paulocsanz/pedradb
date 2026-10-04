# Verification — what is checked, and what is not

This is the annex for the [Verification](../README.md#verification)
paragraph in the README. The README states the result. This file states
what a pair is, what CI actually runs, and what stays outside the proof.

## The claim

A decision kernel is machine-checked when three things agree:

1. The function `rustc` links is the one named in
   `scripts/formal/catalog.json`.
2. Charon + Aeneas extracted that file to Lean, and the extract stamp
   (`formal/aeneas/out/SOURCE.*`) still matches the file's sha256.
3. The Lean file has the named theorem and does not contain `sorry`.

Every one of the **172** catalog pairs is `single_artifact`: the twin is
the kernel. There is no second copy of the decision that can drift from
the binary. **170** of the 172 also carry the three teeth: a production
caller, an `as_is` mutant (the bug the proof refuses), and a test that
plants that mutant and expects failure.

This is not "no bugs". The OS, the disk, rustc, Aeneas, Lean, and Z3 are
not proved. This repository ships the embedded engine and its
operational surface (`pedradb-core`, `-posix`, `-io-uring`, `-sim`,
`-spec`, `-ops`, `rocksdb-compat`, and the bench harnesses). The
distributed building blocks under development — store, raft, sql, http,
replication — are **not in this repository**, so their proofs are not
either; the store/raft refinement kernels (`t1_modelo`, `c1_modelo`)
live in the development tree. Count the pairs yourself: every id in
`scripts/formal/catalog.json` names a kernel file present in this tree —
that self-consistency is what the lint's extract and GAP checks enforce.

## Kernel proofs vs runtime engineering armor

We maintain a strict boundary between what is proved in Lean 4 and what is protected by runtime verification:

1. **Mathematically proved (Lean 4 via Aeneas)**: The 172 pure decision kernels (`*_kernel.rs`). These prove core algebraic invariants in isolation: monotonic sequence ordering, tombstone dominance, compaction disjointness, and write-admission gating.
2. **Guarded by runtime engineering armor (DST, PCT, RAII, Sanitizers)**: The multi-threaded engine runtime (`ConcurrentDb`), concurrency primitives (`RwLock`, atomic sequences, SuperVersion publishing), and physical I/O crates (`pedradb-posix`, `pedradb-io-uring`). These are protected against real-world hardware and OS physics via:
   - **Immediate Linearizability (RFC-0330)**: Point cache negative-hit invalidation and read-lock fallback preventing transient stale reads.
   - **Anti-Hole Crash Consistency (Contract F182)**: RAII seals writing valid NOP frames on aborted write jobs.
   - **Deterministic Simulation Testing (DST)**: Seeded fault injection simulating torn writes and lying fsyncs on real POSIX disks (`PEDRA_SWARM_DISK=1`).

## Where the 172 pairs sit

Counted by the kernel file `rustc` links — regenerate this table with
`python3 -c "import json,collections; c=collections.Counter(p['kernel'].split('/')[-1].rsplit('_kernel.rs',1)[0].rsplit('.rs',1)[0] for p in json.load(open('scripts/formal/catalog.json'))['pairs']); print(sorted(c.items(), key=lambda kv:-kv[1]))"`:

| Kernel file | Pairs | What the decisions are |
|---|---:|---|
| `write_admission` | 20 | Stall, WAL barrier, torn head and tail, directory fsync, empty batch, sequence exhaustion |
| `group_commit` | 17 | Who is in the group, when it publishes, first-committer wins |
| `flush` | 14 | When a memtable is due, per family and globally |
| `scan`, `cf`, `leveling`, `group_window` | 7 each | Scan guard and window; column-family codec; level assignment; group-commit window |
| `compact`, `lookup`, `env_crash`, `wal_state`, `scale` | 6 each | Compaction choice; point lookup; which crashes are legal; WAL state; forecast cuts |
| `changelog`, `cqe`, `recover` | 5 each | Changelog rebuild; io_uring completion codes; torn-tail recovery |
| `lsm_r1`, `properties` | 4 each | No resurrection across levels; D1/R1/T1 predicates |
| `bloom`, `manifest`, `merge`, `probe_order`, `write_ack` | 3 each | Bloom membership; MANIFEST install; merge step; newest-first probe; ack before the barrier |
| `reopen`, `vlog_gc`, `d1_modelo`, `wal_ticket`, `client_axis` | 2 each | Reopen after damage; value-log GC; durability model; WAL ticket; client axis |
| one pair each | 15 | Prefix end, CRC, key pack, batch, lock table, iterator window, magic, ops PITR window, crate root, and the remaining single-decision kernels |

The area table that used to sit in the README was an orientation. Several
of its labels (`product_crown`, `bloom_filter`, `lookup` as a single id,
`properties_kernel`) are not catalog ids. The ids are the `pairs[].id`
strings in `catalog.json`.

## What CI runs

`.github/workflows/ci.yml` executes three independent jobs with dedicated time budgets so test builds never starve verification:

1. **`tests`**: `cargo test -q -p pedradb-core --lib` (hard fail on any test failure).
2. **`formal-lint`**: verifies glue lint and classification debt with ceiling 0:
   ```sh
   python3 scripts/formal/pedra_formal.py --lint > lint.log
   python3 scripts/ci_ratchet.py lint lint.log 0
   ```
3. **`hygiene`**: runs `python3 scripts/check_public_hygiene.py` ensuring zero internal leaks or dangling links.

On the current tree, the test suite passes with **1,325 passed, 4 ignored** (the
README's Verification paragraph carries the live number), and the formal lint
passes with **0 gap, 0 fail** at debt ceiling 0 — the ok-line count moves with
the tree; the invariants are the zeros.

`ci_ratchet.py` exits 1 if any FAIL line is found or if the FAIL count exceeds the debt ceiling (0). The debt ceiling is zero tolerance: any drift or unclassified public surface fails CI.

`scripts/pedra_formal.sh` is the same Python entry with more flags.
GitHub CI uses `--lint` only. Locally:

```sh
./scripts/pedra_formal.sh --lint              # what CI runs
./scripts/pedra_formal.sh --ci                # lint, clones, twins, scripts, models, extract stamps
./scripts/pedra_formal.sh --extract-required  # Charon and Aeneas must be installed
./scripts/lean_extracts.sh --required         # lake build every shipped theorem file
```

`--ci` does not run Verus. No Verus twin is shipped here, so `--verus` is
empty and `--verus-required` fails only when a `verus` binary was
required and is missing.

The pre-commit hook (`.githooks/pre-commit`, installed with
`git config core.hooksPath .githooks`) compares each staged `*_kernel.rs`
to its `SOURCE.*` sha256. A kernel edit that is not restamped does not
commit.

## What each check refuses

| Check | Refuses |
|---|---|
| lint | A catalog `entry` that the listed production file never calls. A `data_fate` pair without a handler, an `as_is` mutant, and a test that calls the entry. |
| clones | A registered clone group that drifted, or two identical production functions left unregistered. |
| twins | A close twin missing the entry, or a decision token (`==`, `0xff`, …) present in the kernel and absent from the twin. |
| models | A Stateright model test failing: recovery, prefix, range, changelog, bloom, scan. |
| extract | A stamp whose sha256 no longer matches the kernel, a Lean file that gained `sorry` or lost a named theorem, or a theorem file that disappeared while its kernel is still shipped. |
| residuals | `residuals.json` kernel-file count, line counts, or the public-fn allowlist drifting from the tree. `glue.db_rs_extracted` stays false: `db.rs` is not extracted. |
| proof vs campaign | A pair that is neither a proof object nor a registered campaign gate. |

`GAP` lines name crates that are not in this tree. They do not fail the
lint. `--strict` fails catalog rows whose status is `absent`; it does not
turn `GAP` into a failure.

## The verified profile

`pedradb_core::verified::profile_report` is the machine-checked tie to
the catalog. The set of kernels marked On equals the 172 pair ids.
Nothing else is claimed On. The test is
`verified_report_matches_catalog`.

The profile opens with `sync=true`, a full WAL fsync, and fail-closed
recovery. The io_uring ring stays off: there is no proved ring model, so
the verified constructors pin the POSIX environment. The catch-up window
is pinned to 0.

## Dynamic checks around the proofs

- **1,325** `pedradb-core` tests on the green CI run, including the
  Stateright models above, codec smoke tests, a WAL durability suite, and
  a concurrent race suite.
- **`pedradb-sim`**: the same engine, with an `Env` that can fail the Nth
  I/O, lie about fsync, short a write, or tear a WAL tail. Same seed, same
  execution. It is fault injection on the real recovery path, not a
  whole-system simulator.
- **RocksDB oracle**: lab-only. The oracle crate is not in this
  repository, and the engine never links RocksDB.

## Reproducing the lint

From a clean checkout, with Python 3:

```sh
python3 scripts/formal/pedra_formal.py --lint
python3 scripts/ci_ratchet.py lint lint.log 0   # after redirecting the lint
```

A drift FAIL names the kernel path and the stamp. Restamp from the source
file the stamp's `path=` line names, then re-run the lint. Do not flip
`glue.db_rs_extracted` to true to silence a row.
