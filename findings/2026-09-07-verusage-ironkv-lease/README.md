# IronKV host program is the Verus term (VeruSAGE IR)

**Date:** 2026-09-07
**Primary sources:**
- Yang, Neamtu, Hawblitzel, Lorch, Lu, *VeruSAGE: A Study of Agent-Based Verification for Rust Systems*, arXiv:2512.18436v2 (15 Apr 2026). PDF: `2512.18436.pdf`.
- `verus-lang/verified-ironkv` README (fetched 2026-09-07): `verified-ironkv-README.md`.

## What the sources actually say

VeruSAGE-Bench extracts 849 Verus proof tasks from eight systems. IronKV (abbr. IR, 118 tasks) is the Verus port of IronFleet IronSHT: a sharded key-value store. The repo states it **only verifies the host program** — the implementation rustc/Verus compile — and does **not** replicate IronFleet's TLA host-in-distributed-system layer.

That is the seL4 price in Verus clothing: the code that runs is the proof term. A twin-cópia beside production is not that class.

## Used this turn

Catalog pair `lease` (`data_fate`, entry `lease_live`): DCS absolute-deadline live check (`lease == 0` immortal; else `now_ms < lease`). F56 death is monotone in the clock — a rewind of `now_ms` reanimates, which is the AS-IS hole (`lease_live_as_is` always true). Production `lease_kernel.rs` is now the Verus term (`single_artifact`). Lemma `lemma_as_is_keeps_expired_live` is the second possibility. Pairs `lease_table` / `lease_next_id` keep the twin until their turn.

## Not claimed

IronKV TLA distributed-system refinement. Dump of `dcs/src/lib.rs` clock/HashMap glue. “somos seL4”. LLM-synthesized proofs (VeruSAGE's 80% agent result is a different question).
