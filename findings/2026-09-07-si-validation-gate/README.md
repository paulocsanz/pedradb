# SI validation gate = the exec the binary calls (arxiv 2606.17182)

**Date:** 2026-09-07
**Primary source:** Khan, *Verified Detection and Prevention of Concurrency Anomalies in Multi-Agent Large Language Model Systems*, arXiv:2606.17182v1 (15 Jun 2026). PDF: `2606.17182v1.pdf`.

## What the paper actually does

Section VI-F / “In-the-loop verification of the SI validation gate”: the deployed `SnapshotIsolationStore::commit` read-set check is **lifted into a Verus exec function**, proved sound and complete against the freshness predicate (`validate`: 5 verified, no assume / admit / external_body), and **the deployed commit now invokes that exact function**. Default SI still misses the no-write stale-read pattern; SSI (`validate_no_write`) closes it. N-way TLC witnesses at |A|∈{2,3}.

This is the same seL4-price move as RFC-0171 / RFC-0174: the `.rs` rustc links is the proof term. Not a twin-cópia.

VerusSync (Lattuada et al., *Verus*, 2025 MSR TR) is the adjacent concurrency DSL: application-level concurrent reasoning; crash = atomic STM as a special case. Pedra still treats `parking_lot` as TCB and proves **clients**.

## Used this turn

Catalog pair `iter_window` (`data_fate`, was twin ≠ kernel): `crates/rocksdb-compat/src/iter_kernel.rs` is now `single_artifact`. `iter_window_keep` is the SI **snapshot-visibility** gate on the CF iterator (do not emit a row `visible_at` hid). Verus `lemma_as_is_emits_hidden` is the second possibility (AS-IS leak). Clone with `merge.rs` stays token-identical on the rustc body.

**Not this turn (rank 6, still unpaid):** N-way `group_validate` of N>2 `OccRead`s — the paper’s |A|∈{2,3} SI commit gate. Pedra’s analogue is ConcurrentDb OCC, already extracted; Lean still only has N≤2 examples.

## Not claimed

“we are seL4”, “TSan is a proof”, “PCT d=2 = ∀π”, dump of `db.rs` / `concurrent.rs` / `parking_lot`.
