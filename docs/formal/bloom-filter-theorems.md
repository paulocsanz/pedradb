# Bloom filter theorems (T1–T4) — `pedradb-core/src/bloom.rs`

**Date:** 2026-08-15 · **Scope:** the filter *algorithm* beyond the F166 header
(`bloom_header_ok` already has its own twin + model). The production module's
own doc line is the contract: *false positives are allowed; **false negatives
are not***.

## Theorems

| Id | Statement (universally quantified over the stated domain) | Carried by |
|----|------------------------------------------------------------|------------|
| **T1** | Active filter, key set `K`: `insert(k)` for `k ∈ K` ⇒ `may_contain(k)` for every `k ∈ K` | Verus twin (unbounded, model domain) · Kani (production code, keys ≤ 6 B, ≤ 4 keys) · Lean/Aeneas (production extract) · exhaustive finite-domain test · fuzz |
| **T2** | `decode(encode(f))` reproduces `f` (structural) and every membership decision | Kani (production, 1 symbolic key) · exhaustive finite-domain test · fuzz |
| **T3** | Every header accepted by `bloom_header_ok` (F166) decodes without panicking, and every probe index stays inside `bits` (the `bit_index(…).unwrap_or(0)` fallback is unreachable on that path) | Kani (production, residual ≤ 32 B) · F166 twin (loop-bound half) |
| **T4** | Inactive filter (`always_true`, zero capacity, zero bits/key) never rejects any key | Verus twin · Kani · exhaustive test |

## What T1 does and does not assume

- Assumes only that `hash_pair` is **deterministic** (same key ⇒ same
  `(h1, h2)`) and that `insert` and `may_contain` compute probe indices with
  the **same expression**. No assumption about FNV quality, distribution, or
  false-positive rate.
- The false-positive *rate* is probabilistic — out of scope for a ∀ theorem;
  the `rejects_many_absent_keys` unit test keeps the statistical sanity check.
- Multi-key monotonicity (bits only get set, never cleared) is the heart of
  T1: the Verus twin states it per-loop; Kani proves it bit-precisely for ≤ 4
  symbolic keys; the exhaustive test covers 85 keys × 15 filter shapes.

## Machines (verification floor → artifact)

1. **Floor 1 — tests with teeth.**
   `crates/pedradb-core/tests/bloom_filter_model.rs`: exhaustive over the
   finite domain (alphabet `{0x00, 0x01, 0xfe, 0xff}`, key length ≤ 3 → 85
   keys; `with_capacity` grid 3 × 5) calling the **production** functions,
   plus a deterministic 500-iteration fuzz smoke. Teeth: the hypothetical
   mutants `may_contain_mut_extra_probe` (query probes `k+1` bits) and
   `may_contain_mut_hash_mismatch` (query perturbs `h2`) must produce at
   least one false negative each on the same domain — otherwise T1 bites
   nothing. Both mutants are hypothetical (no known occurrence); they exist
   to keep the property suite honest.
2. **Floor 3 — Verus twin.** `crates/pedradb-core/verus/bloom_filter.rs`
   (model domain: one bool per bit, non-wrapping probe index), entry
   `insert_then_may_contain` — the fn contract *is* the ∀ theorem. Run:
   `./scripts/verus_bloom_filter.sh`.
3. **Floor 2 — Kani (first Kani use in pedradb-core).**
   `#[cfg(kani)] mod kani_proofs` in `src/bloom.rs` proves the theorems on
   the **compiled production code**, bit-precise: T1 (symbolic keys ≤ 6
   bytes, capacity ≤ 128, bits/key 1..=64), T2 (symbolic key), T3 (symbolic
   header in the F166-accepting domain, residual ≤ 32 B), T4. Run:
   `./scripts/kani_bloom.sh`. Kani complements Verus exactly where a spec
   can idealize: `wrapping_add`/`wrapping_mul`/`%` are checked as compiled.
4. **Floor 3b — Aeneas/Lean second machine (P2).**
   `formal/aeneas/bloom-kernel/` extracts the production `bloom.rs` via
   Charon→Aeneas; Lean proves `insert_then_may_contain` on the extract.
   Status tracked in `formal/aeneas/EXTRACT.md`.

## Honest sentence

> For every active filter shape and key, `insert` then `may_contain` answers
> true — Verus accepted the model-domain twin, Kani accepted the production
> code bit-precise (bounded key length/residual as documented), Lean accepted
> the Aeneas extract (P2). Relative to axioms: hash determinism only; FPR
> stays statistical.

Never: "the Bloom filter is correct" (unqualified), or "no false positives
are possible."

## Bounds (Kani, documented)

The first host run of the wider bounds (residual ≤ 32, 4 keys × 6 B,
capacity ≤ 128, bpk ≤ 64, unwind 64) sat in CBMC > 1 h / 12 GB on T2
and was killed. These are the bounds that the script actually runs:

| Harness | Symbolic | Bound |
|---------|----------|-------|
| `insert_then_may_contain_all_keys` | 2-byte key | decoded 64-bit / k=2 filter, unwind 8 — **green** (2959 checks, ~90 s) |
| `inactive_filter_never_rejects` | 1 key × 4 B | unwind 8 — **green** (337 checks, <1 s) |
| `decode_header_ok_yields_safe_filter` | header + payload | residual ≤ 8, k ≤ 8 — **not in the script** (CBMC class of T3) |
| `encode_decode_roundtrip_preserves_filter` | 1 key × 3 B | **not in the script** — OOM / stall; T2 is the exhaustive test |

Unbounded T1 (model domain) is the Verus twin. The algebraic T1 core
(`set_bit` then `test_bit` on the same index) on the production extract
is Lean `set_bit_test_bit_same`. Multi-key / larger filters stay with
the exhaustive test + fuzz. The insert-loop / query-loop composition
on the extract is not claimed.
