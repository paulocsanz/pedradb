# RFC-0030: Bloom filter ∀-verification — three machines on one core kernel

**Status:** in-progress (P0 done; P1.1–P1.3 done, P1.4 Kani T4 green / T1–T3 running; P2.1 extract green, P2.2 T4 accepted)
**Updated:** 2026-08-15
**Parent:** [`docs/formal-verification-strategies.md`](../formal-verification-strategies.md) ·
theorems page: [`docs/formal/bloom-filter-theorems.md`](../formal/bloom-filter-theorems.md)

---

## Background

- `pedradb-core/src/bloom.rs` documents its own contract — *"false positives
  are allowed; **false negatives are not**"* — but until now only the header
  bound (`bloom_header_ok`, F166) had a Verus twin and a Stateright model.
  The filter algorithm itself (`insert` / `may_contain` / `encode` /
  `decode`) had unit tests only; the module's central theorem was prose.
- The house already runs a multi-floor verification portfolio (tests with
  AS-IS teeth → Stateright → Verus twins → Aeneas/Lean extract), documented
  in `docs/formal-verification-strategies.md` (updated 2026-08-15).
  **Kani is installed on this host (`cargo-kani` 0.67.0) but had zero use in
  `pedradb-core`** — the floor "bit-precise proof of the *compiled*
  production code" was empty for the core crate.
- Why now: the 2026 survey (formal-verification-strategies §2.2/§2.4) already
  concluded Verus and Kani are complements — Kani catches `wrapping_add` /
  `%` semantics that a Verus spec written over mathematical `int` can
  idealize away. The bloom filter is exactly a `wrapping_mul` + modulo kernel,
  and the `verify-rust-std` campaign runs Kani on the real stdlib at scale,
  validating the approach on this class of code.

## Problems This Solves

- **Problem:** the no-false-negatives contract of a production lookup-path
  kernel is enforced only by sampled tests (`no_false_negatives` inserts 100
  keys), not by any ∀ statement on any machine.
- **Problem:** `pedradb-core` has no Kani floor; every wrap/module arithmetic
  argument in the crate rests on review plus tests.
- **Problem:** the bloom twins (Verus) could drift from production — a twin
  nobody cross-checks on the production code is a second implementation, not
  a proof (the "twin risk" the strategies doc carries).

## Proposed Solution

Prove the same four theorems on **independent machines**, so agreement is
cross-validation rather than one trusted tool:

- **T1 (no false negatives):** active filter; `insert(k)` ⇒ `may_contain(k)`.
  Assumes only hash determinism + insert/query index agreement — nothing
  about FNV quality. Multi-key monotonicity (bits only OR) is the proof core.
- **T2 (roundtrip):** `decode(encode(f))` reproduces `f` and its decisions.
- **T3 (memory safety under F166):** every header `bloom_header_ok` accepts
  decodes without panic and every probe index stays inside `bits` — turning
  F166's "bounded loop" into "bounded *and in-bounds*" and proving the
  `bit_index(..).unwrap_or(0)` fallback unreachable on that path.
- **T4 (inactive never rejects):** `!is_active ⇒ may_contain = true` ∀ key.

Machines: (1) exhaustive finite-domain tests + deterministic fuzz + mutant
teeth on the **production** fns; (2) Kani harnesses on the **compiled
production** fns (bit-precise wrap/modulo — the empty floor); (3) Verus twin
with the ∀ theorem as an `ensures` (model domain: `Vec<bool>` bits,
non-wrapping index form; unbounded k/nbits); (4) P2: Aeneas/Lean extract of
the production file as a second proof assistant.

## Delivery slices

### P0 — Spec page + Verus twin (unbounded T1/T4)

- [x] **P0.1** Theorems page `docs/formal/bloom-filter-theorems.md` (T1–T4,
      domains, axioms, bounds, honest sentence) — status: `done`
- [x] **P0.2** Verus twin `crates/pedradb-core/verus/bloom_filter.rs`: entry
      `insert_then_may_contain` (fn contract is the ∀ theorem), `probe_index`
      with `r < nbits`, `may_contain_inactive` (T4) —
      `./scripts/verus_bloom_filter.sh` green: **7 verified, 0 errors** —
      status: `done`
- [x] **P0.3** `scripts/verus_bloom_filter.sh` + catalog entries
      (`bloom_insert`, `bloom_may_contain`) — status: `done`

### P1 — Tests with teeth + Kani on production (first Kani in the core)

- [x] **P1.1** Hypothetical teeth mutants in `src/bloom.rs`
      (`may_contain_mut_extra_probe`, `may_contain_mut_hash_mismatch`) —
      status: `done`
- [x] **P1.2** `tests/bloom_filter_model.rs`: exhaustive finite domain
      (alphabet `{0x00,0x01,0xfe,0xff}` ≤ 3 bytes → 85 keys × 15 filter
      shapes) on production fns + T1–T4 + both mutants must bite +
      500-iteration deterministic fuzz — 7/7 green — status: `done`
- [x] **P1.3** `#[cfg(kani)]` harnesses in `src/bloom.rs` (T1–T4, bounds in
      the theorems page) + `check-cfg` for `kani` in `Cargo.toml` +
      `scripts/kani_bloom.sh` (residual-friendly, `--required` mode) —
      status: `done` (harness green run pending — see Risks)
- [ ] **P1.4** `cargo kani -p pedradb-core` full green pass; wire
      `kani_bloom.sh` into the docs and (optionally) CI — status: `doing`
      (T4 SUCCESSFUL, 337 checks / 0.18 s. First T2 attempt at residual≤32 /
      capacity≤128 sat in CBMC >1 h / 12 GB and was killed; bounds reduced
      per theorems page. T1–T3 rerun in flight.)

### P2 — Second proof assistant (Aeneas → Lean) on the production file

- [x] **P2.1** `formal/aeneas/bloom-kernel/` include-crate (`[lib] path` →
      `crates/pedradb-core/src/bloom.rs`); `scripts/aeneas_bloom.sh`
      (Charon `--preset=aeneas` → LLBC → Aeneas → Lean). Production
      rewrites so the extract typechecks (`bit_index`, `is_active`,
      `with_capacity`) — status: `done`
- [ ] **P2.2** Lean theorems on the extract: T4 accepted
      (`may_contain_nbits_zero`, `may_contain_k_zero`,
      `always_true_never_rejects`); loop-level T1 (`insert` then
      `may_contain`) and T2 still open — `scripts/lean_bloom.sh --required`
      — status: `doing`
- [x] **P2.3** Glue: `EXTRACT.md` bloom section, `pedra_formal.py`
      SOURCE.bloom + extract lint, strategies doc §6 row, this status
      table — status: `done`

## Status (living — update with every change)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Theorems page (T1–T4, bounds, honest sentence) | done | this change | 2026-08-15 |
| P0.2 | p0 | Verus twin green (7 verified) | done | this change | 2026-08-15 |
| P0.3 | p0 | verus script + catalog entries | done | this change | 2026-08-15 |
| P1.1 | p1 | teeth mutants (hypothetical, labeled) | done | this change | 2026-08-15 |
| P1.2 | p1 | exhaustive domain + fuzz + teeth green (7/7) | done | this change | 2026-08-15 |
| P1.3 | p1 | Kani harnesses + script (residual-friendly) | done | this change | 2026-08-15 |
| P1.4 | p1 | Kani full green pass + doc/CI wiring | doing | T4 green; T1–T3 rerun | 2026-08-15 |
| P2.1 | p2 | Aeneas bloom include-crate + extract | done | this change | 2026-08-15 |
| P2.2 | p2 | Lean T4 on the extract (T1/T2 loops open) | doing | this change | 2026-08-15 |
| P2.3 | p2 | glue (EXTRACT/pedra_formal/docs) | done | this change | 2026-08-15 |

## Acceptance Criteria

- **Tests:** `cargo test -p pedradb-core --test bloom_filter_model` (7 tests:
  T1–T4 exhaustive, 2 mutant-bite teeth, fuzz); existing `bloom` unit tests
  and `bloom_model` (F166) untouched and green.
- **Proofs:** `./scripts/verus_bloom_filter.sh` exit 0 (no `sorry` analog in
  Verus: 7 verified / 0 errors); `./scripts/kani_bloom.sh --required` exit 0
  on a kani-installed host; P2: `./scripts/lean_bloom.sh --required` exit 0
  with no `sorry` in the Lean file.
- **Teeth:** each mutant is caught by at least one named test
  (`teeth_extra_probe_mutant_bites`, `teeth_hash_mismatch_mutant_bites`).
- **Telemetry:** none — proofs and tests; no runtime code paths changed
  (mutants are additive `pub fn`s used only by tests).
- **Documentation:** theorems page (P0.1), this RFC, strategies doc §6 rows,
  `EXTRACT.md`/`PINS.md` (P2).
- **Screenshots:** none — backend/proofs only.

## Out of scope

- False-positive **rate** (probabilistic; stays with the statistical unit
  test `rejects_many_absent_keys` — not a ∀ property).
- Verifying FNV-1a quality/distribution, or swapping the hash (Monkey-style
  allocation is `docs/engine-landscape-and-ideal-path.md` territory).
- Verus/Creusot/hax tool swap debate (settled in
  formal-verification-strategies §2.3/§2.4; this RFC follows it).
- Kani on other core kernels (`key.rs`, `vlog.rs` codecs) — follow-up slices
  once the P1.4 green pass establishes the build cost on this host.
- IronFleet-style refinement of the whole engine.

## Risks

- **Kani build time:** first `cargo kani -p pedradb-core` on this host has
  been compiling > 20 min (task-agnostic workspace build). Mitigation: the
  script runs all four harnesses in one invocation; if wall time stays
  painful, restrict to `--harness` selection or move the gate to
  `--required` on a dedicated machine (same policy as Verus in CI).
- **Twin drift:** mitigated by catalog `twin_kind: close` token checks
  (`pedra_formal.py`) + P2 extract being the production file itself.
- **Aeneas extract edges** (`to_le_bytes`, `Vec` ops) may become axioms —
  recorded in `EXTRACT.md` per house norm; axiom-free is a goal, not a gate.

## Honest sentence (what we may say when P1 is green)

> For every active filter and key, `insert` then `may_contain` answers true:
> Verus accepted the model-domain twin (unbounded k, nbits ≤ 2³²−1), Kani
> accepted the compiled production code bit-precise (bounds documented), and
> the exhaustive domain + fuzz + mutants passed on the production fns.
> Relative to axioms: hash determinism only.

Never: "the Bloom filter is correct" (unqualified), or "false positives are
impossible."
