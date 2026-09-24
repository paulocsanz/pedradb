# Aeneas / Lean recipes (Pedra)

Pins: Charon `0.1.232`, Aeneas `daa85d7`, Lean `4.31.0`. `glue.db_rs_extracted=false`.

## Atom

Production total fn (`match`/`while`/`if`, no `impl Iterator` return). Catalog
`entry` + `as_is` dente. Extract crate `[lib] path` = production or `#[path]`
shim of siblings. `scripts/aeneas_<stamp>.sh --required` stamps
`formal/aeneas/out/SOURCE.<stamp>` = `shasum -a 256` of the live file (posix:
git HEAD if dirty). Pedra theorems in `formal/aeneas/lean/<Name>.lean` over
the generated `*Kernel.lean` (symlink to `out/lean/` when possible). `unfold`
the def; `rfl` / `simp` / `native_decide`. No substring `sorry` in Pedra
theorem files (Aeneas.Std may still have `sorry`).

`--start-from` catalog entries. `--exclude` non-catalog bottoms (e.g. Pattern).
`RUSTFLAGS='--cfg test'` if as-is is `#[cfg(test)]`. Partial `.lean` is not
an extract. `proof_depth`: close + matching SOURCE ⇒ extract; **model stays
model** even if the file is extracted. A `verus!` toy enum / `u64` /
`Seq<u8>` stand-in while rustc has `&[u8]` / `Bound` / `key::ValueType`
is that model — not the extracted rustc body, not last-wins. Payment:
delete the stand-in; Aeneas of the rustc types. `_body!` over different
types is the same lie.

Iterator extra generated fields (`map`/`filter`/`collect`/`all`) lake-red —
strip in `aeneas_*.sh`. Bool `if` over Result can parse as Prop `ite` — use
`match true/false`. Loops are `ControlFlow` `done`/`cont`; `partial_fixpoint`
needs `LawfulBEq` + `native_decide` on examples, not `rfl` of the whole loop.

## Dual-unfold (compose)

A same-file `unfold` of only the caller does **not** satisfy composition.
Unfold **caller and callee** on a representative input, plus as-is or other
branch. The caller Lean unfolds is the **plan fn the glue handler calls**
(`occ_batch_plan`, `wal_commit_plan`) — glue methods in `db.rs` /
`concurrent.rs` are never extracted. `native_decide` of the plan without
`unfold` is **not** compose. Unfolding only the callee (`group_validate`
without `occ_batch_plan`) does not pay `validate_occ_batch` /
`commit_ops_with` (`concurrency.md`, `script.md`). Cross-lib: new
`Compose*.lean` importing both Kernels; add to `lakefile.toml` and
`scripts/lean_extracts.sh` `COMPOSE=`. Importing two Kernels that stamp the
same discriminant instance can lake-red (measured: T1Modelo+Txn) — then
unfold the shim copy and restate the callee on its own lib, named in
`EXTRACT.md`.

`lean_extracts.sh --required`: missing file, `sorry`, or `lake` red = fail.

## Charon will not take

Do not rewrite the engine *except* Isolated-style index `while` for a catalog
fn the pin CFailures as Iterator (covering 2026-09-07). Nested `Vec.push`
loops may still hole — patch the generated body in `aeneas_*.sh` to a
`loop`/`ControlFlow` def (same semantics). Do not invent production `fn`s
the catalog names that never existed (`l28_tcp_*_ok`).
