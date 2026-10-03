-- Theorems over Aeneas extract of store compact_kernel.rs
-- (RFC-0002 P13 / F27 / F28 / RFC-0100 / RFC-0109). Payment is the linked
-- rustc bodies; the former cfg(verus_keep_ghost) stand-in was deleted.
-- Fail-closed: this file must not contain a hole.
import Aeneas
import StoreCompactKernel
open Aeneas.Std Result
open pedra_aeneas_store_compact_kernel

/-- F28: an offline peer still counts in the compact watermark. -/
theorem peer_counts_offline_still_counts :
    peer_counts_for_compact false = ok true := by
  rfl

/-- F28: a participating peer counts too. -/
theorem peer_counts_participating_counts :
    peer_counts_for_compact true = ok true := by
  rfl

/-- AS-IS F28 dente: only live peers — compact past offline applied. -/
theorem peer_counts_as_is_skips_offline :
    peer_counts_for_compact_as_is false = ok false := by
  rfl

/-- F27: nothing durable at applied 0 — no compact. -/
theorem compact_ready_zero_not_ready :
    compact_ready (0#u64) = ok false := by
  unfold compact_ready
  rfl

/-- F27: durable entries make compact ready. -/
theorem compact_ready_positive_ready :
    compact_ready (3#u64) = ok true := by
  unfold compact_ready
  rfl

/-- F27: the entry must be in the log (through 0 refused). -/
theorem may_compact_through_zero_false :
    may_compact_through 0#u64 0#u64 1#u64 = ok false := by
  unfold may_compact_through
  rfl

/-- F27: missing term at through (entry absent) refuses. -/
theorem may_compact_through_missing_term_refuses :
    may_compact_through 0#u64 5#u64 0#u64 = ok false := by
  unfold may_compact_through
  rfl

/-- F27: a present entry with a term may compact. -/
theorem may_compact_through_present_term_allows :
    may_compact_through 0#u64 5#u64 2#u64 = ok true := by
  unfold may_compact_through
  rfl

/-- F27: already covered by the snapshot — refuse. -/
theorem may_compact_through_snapshot_covered_refuses :
    may_compact_through 5#u64 5#u64 2#u64 = ok false := by
  unfold may_compact_through
  rfl

/-- AS-IS F27 dente: compacts even when the entry is missing. -/
theorem may_compact_through_as_is_missing_term_allows :
    may_compact_through_as_is 0#u64 5#u64 0#u64 = ok true := by
  unfold may_compact_through_as_is
  rfl

/-- Floor after compact through n is n+1. -/
theorem compact_index_floor_advances :
    compact_index_floor (7#u64) = ok 8#u64 := by
  unfold compact_index_floor
  have h : core.num.U64.saturating_add 7#u64 1#u64 = 8#u64 := by native_decide
  simp [h]

/-- u64::MAX saturates — floor never wraps to 0. -/
theorem compact_index_floor_max_saturates :
    compact_index_floor (18446744073709551615#u64)
      = ok 18446744073709551615#u64 := by
  unfold compact_index_floor
  have h : core.num.U64.saturating_add 18446744073709551615#u64 1#u64
      = 18446744073709551615#u64 := by native_decide
  simp [h]

/-- RFC-0100/0109: compact stops before an un-left joint. -/
theorem compact_through_unleft_caps_below_joint :
    compact_through_unleft (5#u64) (some 3#u64) = ok 2#u64 := by
  unfold compact_through_unleft
  have hgt : (3#u64 > 0#u64) = true := by native_decide
  have hle : (3#u64 <= 5#u64) = true := by native_decide
  have hsub : core.num.U64.saturating_sub 3#u64 1#u64 = 2#u64 := by native_decide
  simp [hgt, hle, hsub]

/-- Joint at the exact through index also caps (through j-1). -/
theorem compact_through_unleft_joint_at_through_caps :
    compact_through_unleft (5#u64) (some 5#u64) = ok 4#u64 := by
  unfold compact_through_unleft
  have hgt : (5#u64 > 0#u64) = true := by native_decide
  have hsub : core.num.U64.saturating_sub 5#u64 1#u64 = 4#u64 := by native_decide
  simp [hgt, hsub]

/-- A joint past through does not cap (not applied yet). -/
theorem compact_through_unleft_joint_past_through_no_cap :
    compact_through_unleft (5#u64) (some 6#u64) = ok 5#u64 := by
  unfold compact_through_unleft
  have hgt : (6#u64 > 0#u64) = true := by native_decide
  have hle : (6#u64 <= 5#u64) = false := by native_decide
  simp [hle]

/-- No joint: through unchanged. -/
theorem compact_through_unleft_none_no_cap :
    compact_through_unleft (5#u64) none = ok 5#u64 := by
  rfl

/-- A zero joint is no joint (membership equal already). -/
theorem compact_through_unleft_zero_joint_no_cap :
    compact_through_unleft (5#u64) (some 0#u64) = ok 5#u64 := by
  unfold compact_through_unleft
  simp

/-- AS-IS dente: compact past the un-left joint (the 0096/0100 hole). -/
theorem compact_through_unleft_as_is_past_joint :
    compact_through_unleft_as_is (5#u64) (some 3#u64) = ok 5#u64 := by
  rfl

/-- RFC-0212 P2.1 (store-compact cadence, atom `catalog:compact_unleft`):
    compaction NEVER goes past an applied-but-un-left joint — the
    through index is EXACTLY `joint - 1` when a live joint sits at
    or below it, and `through` otherwise — fate forall over the
    extracted body (RFC-0100: a hidden C-old,new lies to later
    readers); the AS-IS mutant compacts straight through the joint
    (the lie the DST plant
    `compact_through_unleft_on_live_queued_is_not_ok` refutes). -/
theorem compact_through_unleft_fate_iff :
    ∀ (through : U64) (unleft_joint : Option U64) (v : U64),
      (compact_through_unleft through unleft_joint = ok v) ↔
        ((unleft_joint = none ∧ v = through)
          ∨ (∃ j : U64, unleft_joint = some j ∧ j > 0#u64 ∧ j <= through
              ∧ v = core.num.U64.saturating_sub j 1#u64)
          ∨ (∃ j : U64, unleft_joint = some j
              ∧ (¬ (j > 0#u64) ∨ ¬ (j <= through))
              ∧ v = through)) := by
  intro through unleft_joint v
  cases unleft_joint with
  | none =>
    simp [compact_through_unleft]
    exact eq_comm
  | some j =>
    unfold compact_through_unleft
    by_cases hpos : j > 0#u64
    · by_cases hle : j <= through
      · simp [hpos, hle]
        scalar_tac
      · simp [hpos, hle]
        scalar_tac
    · simp [hpos]
      scalar_tac

/-- RFC-0218 P1.1 1/10 (átomo `catalog:compact`, entrada
    `may_compact_through`): a permissão de compactar através de um
    índice é EXATAMENTE a árvore citada — recusa through zero, recusa
    coberto pelo snapshot, recusa term zero; autoriza só com os três
    gates abertos. O AS-IS nem olha o term (compacta através de líder
    de term zero — dente plantado). -/
theorem may_compact_through_fate_iff :
    ∀ (snapshot_index : U64) (through : U64) (term_at_through : U64)
      (v : Bool),
      (may_compact_through snapshot_index through term_at_through = ok v) ↔
        ((through = 0#u64 ∧ v = false)
          ∨ (¬(through = 0#u64) ∧ through <= snapshot_index ∧ v = false)
          ∨ (¬(through = 0#u64) ∧ ¬(through <= snapshot_index)
              ∧ term_at_through = 0#u64 ∧ v = false)
          ∨ (¬(through = 0#u64) ∧ ¬(through <= snapshot_index)
              ∧ ¬(term_at_through = 0#u64) ∧ v = true)) := by
  intro snapshot_index through term_at_through v
  constructor
  · intro hval
    unfold may_compact_through at hval
    split at hval
    · next h1 =>
      injection hval with hv
      exact Or.inl ⟨h1, hv.symm⟩
    · next h1 =>
      split at hval
      · next h2 =>
        injection hval with hv
        exact Or.inr (Or.inl ⟨h1, h2, hv.symm⟩)
      · next h2 =>
        split at hval
        · next h3 =>
          injection hval with hv
          exact Or.inr (Or.inr (Or.inl ⟨h1, h2, h3, hv.symm⟩))
        · next h3 =>
          injection hval with hv
          exact Or.inr (Or.inr (Or.inr ⟨h1, h2, h3, hv.symm⟩))
  · rintro (⟨h1, hv⟩ | ⟨h1, h2, hv⟩ | ⟨h1, h2, h3, hv⟩ |
      ⟨h1, h2, h3, hv⟩)
    · subst hv
      unfold may_compact_through
      rw [if_pos h1]
    · subst hv
      unfold may_compact_through
      rw [if_neg h1, if_pos h2]
    · subst hv
      unfold may_compact_through
      rw [if_neg h1, if_neg h2, if_pos h3]
    · subst hv
      unfold may_compact_through
      rw [if_neg h1, if_neg h2, if_neg h3]

/-- RFC-0218 P1.1 2/10 (átomo `catalog:compact_floor`, entrada
    `compact_index_floor`): o piso pós-compactação é EXATAMENTE a
    soma saturada citada — through + 1 sem nunca envolver para zero
    (o AS-IS devolve through e re-requisita o índice já compactado —
    dente plantado). -/
theorem compact_index_floor_fate_iff :
    ∀ (through : U64) (r : U64),
      (compact_index_floor through = ok r) ↔
        (core.num.U64.saturating_add through 1#u64 = r) := by
  intro through r
  constructor
  · intro hval
    unfold compact_index_floor at hval
    injection hval with hv
  · intro hv
    unfold compact_index_floor
    rw [hv]

/-- RFC-0218 P1.1 3/10 (átomo `catalog:compact_peer_counts`, entrada
    `peer_counts_for_compact`): a contagem de pares para o watermark
    de compactação é EXATAMENTE a constante citada true — um par
    offline ainda conta (o quorum não encolhe com queda). O AS-IS
    conta só quem participa (watermark congelável — dente plantado). -/
theorem peer_counts_for_compact_fate_iff :
    ∀ (is_participating : Bool) (v : Bool),
      (peer_counts_for_compact is_participating = ok v) ↔ (v = true) := by
  intro is_participating v
  constructor
  · intro hval
    unfold peer_counts_for_compact at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P1.1 4/10 (átomo `catalog:compact_ready`, entrada
    `compact_ready`): pronto-para-compactar é EXATAMENTE o lift citado
    `min_applied > 0` (decide) — zero aplicado não compacta nada.
    O AS-IS é a constante true (compacta com zero — dente plantado). -/
theorem compact_ready_fate_iff :
    ∀ (m : U64) (v : Bool),
      (compact_ready m = ok v) ↔ (v = decide (m > 0#u64)) := by
  intro m v
  constructor
  · intro hval
    unfold compact_ready at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl
