# Pedra formal glue

Not a prover. Drift glue for a **single-artifact** tree: the proof object is
the shipped kernel itself (`twin == kernel`), extracted with Charon/Aeneas
into Lean, with hand-written theorem files and Stateright models on top.
The checker refuses silent drift between the production kernel, the
catalog, the extract stamps, and the theorem files.

```sh
./scripts/pedra_formal.sh           # same as --ci
./scripts/pedra_formal.sh --ci      # lint + clones + twins + scripts + models + extract (no Verus)
./scripts/pedra_formal.sh --lint    # fast path, no cargo
./scripts/pedra_formal.sh --all     # --ci plus Verus when installed
./scripts/pedra_formal.sh --strict  # also fail on catalog status=absent rows
./scripts/pedra_formal.sh --extract          # stamps + Lean; Charon rebuild if installed
./scripts/pedra_formal.sh --extract-required # Charon/Aeneas must be installed
./scripts/lean_extracts.sh --required        # lake build every shipped theorem file
```

Exit code 1 on any `FAIL`. `GAP` rows do not fail (they name what is not in
this tree); `--strict` does not change that.

## Checks

| Check | What it refuses |
|-------|-----------------|
| **lint** | A kernel whose `entry` is never called from the listed production file. `data_fate` kernels also require named `handlers`, an `as_is` mutant in the kernel file, and a DST plant (`dst_plant.file` + `dst_plant.test` calling `entry(`). The RFC-0152 raft→store `live_callers` wiring is a no-op here (raft/store crates are not shipped) and reports good. |
| **clones** | A registered clone group (`iter_window_merge`, `bench_gap_get_cost`) drifting, a token-identical production fn duplicated across the frozen kernels but left unregistered, or any identical production pair missing from the catalog entirely (test-module fns exempt). |
| **twins** | Missing `twin_kind`; a **close** twin without the entry `fn`; kernel decision tokens (`==`, `0xff`, …) missing from the twin. Single-artifact pairs point the twin at the kernel, so this is a self-consistency check on the catalog row; `test_twin_mutation.py` still injects drift and requires FAIL naming the pair. |
| **scripts** | A `scripts/verus_*.sh` not listed in `catalog.json` (none are shipped; vacuous here). |
| **models** | Stateright model tests in `pedradb-core` (`recover_model`, `recover_choose`, `prefix_model`, `range_model`, `changelog_model`, `bloom_model`, `scan_model`, `bloom_filter_model`) failing. |
| **extract** | `.githooks/pre-commit` (install once per clone: `git config core.hooksPath .githooks`) missing or no longer checking stamps; a `formal/aeneas/out/SOURCE.*` whose `sha256=` no longer matches the shipped kernel (re-run `./scripts/aeneas_<k>.sh`); a regenerated `out/lean/*Kernel.lean` whose marker def disappeared; a theorem file in `formal/aeneas/lean/` containing `sorry`, losing a named theorem, or going missing while its kernel is shipped. `lean_wal_apply_reopen.sh` lake-builds `Reopen` + `WalRecover`; `lean_extracts.sh` (under `--extract-required`) lake-builds every shipped theorem file and re-checks the derived cost annotations. |
| **tcb freeze** | Any new `*_kernel.rs` that is not a catalog pair, a registered clone, or allowlisted; the kernel fn surface (`pub` fns must be `entry` / `as_is` / `_as_is` / `_spec` / clone or in `glue.kernel_fn_allowlist`) growing without a glue update. |
| **residuals freeze** | `residuals.json` glue counts (kernel files/loc, handler loc, proof_depth, data_fate) regressing; a residual row losing id/title/class/owner/close; a never-floor id disappearing; an unsafe-island row missing for a shipped crate. |
| **proof depth** | `scripts/ratchet/close_proofs.tsv` losing theorem/forall/statement rows or depth regressing (`atom` → `close` → `extract` → `model`). |
| **proof vs campaign** | A catalog pair that is neither a proof object nor a registered campaign gate. |
| **class scan** | RFC-0157: `fdatasync`/`fsync`/`fcntl`/`fallocate`/`posix_fadvise` call sites without their gate, CAPI-length params, CQE routing — waivers must name a registered residual id. |
| **verus** | Optional; no Verus twins are shipped in this tree, so `--verus` is vacuous and `--verus-required` only fails if a `verus` binary is expected but missing. |

`twin_kind` is `close` (entry is in the twin), `atom` (smaller `atom` fn
only), or `model` (same name, stand-in domain such as `u64` for `&[u8]`).

## Files

- `catalog.json` — 151 pairs, every one `single_artifact` (twin == kernel).
- `residuals.json` — frozen glue counts, per-kernel fn allowlists, residual rows.
- `ratchet/close_proofs.tsv` — closed-proof registry (depth ratchet).
- `ratchet/host_anchors.tsv`, `ratchet/derive_count_annotations.py` — cost-annotation sync.
- `formal/aeneas/out/SOURCE.<k>` — sha256 stamp of the extracted kernel.
- `formal/aeneas/lean/<K>.lean` — theorem files; `lakefile.toml` targets only what is shipped.

## Not in this tree

The raft, store, stream, http, journal, replicate, fold, world and
recipes kernels are not shipped here; their catalog rows, extract stamps
and theorem files degrade to `GAP` rows naming the unshipped crate. The
`scale` extract is intentionally partial (`write_forecast_cut`,
`WriteGrowth::token`, `WriteStaticCut::token` stay opaque to Aeneas); no
shipped theorem touches those fns.

## In-flight rows

`--ci` also fails on the campaign's in-flight kernel rows (kernels landed
in the engine without their glue row yet). Those fails are mirrored from
the main tree on purpose: this tree adds none of its own. `--lint` is the
fast way to see them without cargo.
