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

Every one of the **151** catalog pairs is `single_artifact`: the twin is
the kernel. There is no second copy of the decision that can drift from
the binary. **132** of the 151 also carry the three teeth: a production
caller, an `as_is` mutant (the bug the proof refuses), and a test that
plants that mutant and expects failure.

This is not "no bugs". The OS, the disk, rustc, Aeneas, Lean, and Z3 are
not proved. Store, raft, and the SQL layer are not in this repository, so
their proofs are not either. `t1_modelo` (store atomicity refinement) and
`c1_modelo` (raft quorum refinement) live in those crates. The public
verified report claims only the 151 shipped pair ids.

## Where the 151 pairs sit

Counted by the kernel file `rustc` links:

| Kernel file | Pairs | What the decisions are |
|---|---:|---|
| `write_admission` | 18 | Stall, WAL barrier, torn head and tail, directory fsync, empty batch, sequence exhaustion |
| `group_commit` | 16 | Who is in the group, when it publishes, first-committer wins |
| `flush` | 11 | When a memtable is due, per family and globally |
| `scan`, `cf` | 7 each | Scan guard and window; column-family encode, decode, and SST tag |
| `changelog`, `compact`, `lookup`, `env_crash`, `wal_state`, `scale` | 6 each | Changelog rebuild; compaction choice; point lookup; which crashes are legal; WAL state; forecast cuts |
| `recover`, `cqe` | 5 each | Torn-tail recovery; io_uring completion codes |
| `merge`, `properties`, `lsm_r1` | 4 each | Merge step; D1/R1/T1 predicates; no resurrection across levels |
| `bloom`, `manifest`, `probe_order`, `write_ack` | 3 each | Bloom membership; MANIFEST install; newest-first probe; ack before the barrier |
| `reopen`, `vlog_gc`, `d1_modelo` | 2 each | Reopen after damage; value-log GC; durability model |
| one pair each | 9 | Prefix end, CRC, key pack, batch, lock table, iterator window, magic, ops PITR window, crate root |

The area table that used to sit in the README was an orientation. Several
of its labels (`product_crown`, `bloom_filter`, `lookup` as a single id,
`properties_kernel`) are not catalog ids. The ids are the `pairs[].id`
strings in `catalog.json`.

## What CI runs

`.github/workflows/ci.yml` does two things, in order:

```sh
cargo test -q -p pedradb-core --lib
python3 scripts/formal/pedra_formal.py --lint > lint.log
python3 scripts/ci_ratchet.py lint lint.log 107
```

On the green run for `eeae662` the test line was **1,094 passed, 4
ignored**. The lint line was **0 FAIL**.

`ci_ratchet.py` exits 1 if any FAIL line contains `drift`, and if the
FAIL count is above 107. A non-drift FAIL under that ceiling still passes
the workflow. The ceiling is a debt cap, not a claim that new failures
are free. The current count is zero.

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
the catalog. The set of kernels marked On equals the 151 pair ids.
Nothing else is claimed On. The test is
`verified_report_matches_catalog`.

The profile opens with `sync=true`, a full WAL fsync, and fail-closed
recovery. The io_uring ring stays off: there is no proved ring model, so
the verified constructors pin the POSIX environment. The catch-up window
is pinned to 0.

## Dynamic checks around the proofs

- **1,094** `pedradb-core` tests on the green CI run, including the
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
python3 scripts/ci_ratchet.py lint lint.log 107   # after redirecting the lint
```

A drift FAIL names the kernel path and the stamp. Restamp from the source
file the stamp's `path=` line names, then re-run the lint. Do not flip
`glue.db_rs_extracted` to true to silence a row.
