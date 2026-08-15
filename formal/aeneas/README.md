# Aeneas extract (vote kernel)

Second machine for [`vote_kernel.rs`](../../crates/pedradb-raft/src/vote_kernel.rs).
The crate in [`vote-kernel/`](vote-kernel/) sets `[lib] path` to the
production `vote_kernel.rs`. There is no hand-written Lean twin.

```sh
# Always: the include crate must compile (same rustc as the workspace).
cargo test --manifest-path formal/aeneas/vote-kernel/Cargo.toml

# When Charon + Aeneas are on PATH (see PINS.md):
./scripts/aeneas_vote.sh
```

`./scripts/pedra_formal.sh --ci` runs the extract-crate `cargo test` and,
if `lake` is on PATH, `lake build Vote`.
`--extract` / `--extract-required` also re-run Charon+Aeneas.

Lean accepted the theorems in [`EXTRACT.md`](EXTRACT.md).
Those are theorems of **extracted terms**, not of persist or the Raft loop.
