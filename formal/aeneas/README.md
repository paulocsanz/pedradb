# Aeneas extracts (public PedraDB kernels)

Production kernel files under `crates/pedradb-core` (and ops / posix /
io-uring / compat / spec) are the proof term. Each `formal/aeneas/<k>-kernel`
crate sets `[lib] path` at that file. Charon+Aeneas emit Lean in
`formal/aeneas/out/lean`; theorems live in `formal/aeneas/lean`.

```sh
# Crate that rustc already linked still compiles as an extract crate:
cargo test --manifest-path formal/aeneas/bloom-kernel/Cargo.toml

# When Charon + Aeneas are on PATH (see PINS.md):
./scripts/aeneas_bloom.sh
```

Raft / store / world extracts are not in this repository.
