-- Cross-lib: the REAL-TCP removal protocol (RFC-0210 P2.2) —
-- remove → left ∧ high-water preserved, COMPOSED over registered
-- atoms. Registration rule: a row needs a single catalog pair/entry;
-- this composition spans the two atoms `catalog:l28_tcp_left` ×
-- `catalog:l28_tcp_hw` (and the ctor walks them in one pass) — same
-- reason the other compose libs carry no row. No holes in this file.
import Aeneas
import L28
open Aeneas.Std Result
open pedra_aeneas_l28_kernel

/-! ### The removal protocol (registered atoms `catalog:l28_tcp_left`
× `catalog:l28_tcp_hw`) -/

/-- RFC-0210 P2.2: the REMOVAL PROTOCOL composed — after
    `remove_member` lands on disk, the removed member is LEFT
    exactly when the on-disk fate says left (atom
    `catalog:l28_tcp_left`), AND the high-water mark is PRESERVED
    exactly when the durable fate says so (atom `catalog:l28_tcp_hw`).
    Each conjunct is the registered atom specialized; the pair is
    the invariant a surviving voter's TCP ctor walks after the
    removal. -/
theorem l28_removal_protocol_fate_composed :
    ∀ (bl bh : Bool),
      ((l28_tcp_left_ok bl = ok true) ↔ (bl = true))
        ∧ ((l28_tcp_hw_ok bh = ok true) ↔ (bh = true)) := by
  intro bl bh
  refine ⟨?_, ?_⟩
  · constructor
    · intro h
      rcases (l28_tcp_left_ok_fate_iff bl true).mp h with
        ⟨_, hP⟩ | ⟨hfalse, _⟩
      · exact hP
      · exact absurd hfalse (by simp)
    · intro hP
      exact (l28_tcp_left_ok_fate_iff bl true).mpr (Or.inl ⟨rfl, hP⟩)
  · constructor
    · intro h
      rcases (l28_tcp_hw_ok_fate_iff bh true).mp h with
        ⟨_, hP⟩ | ⟨hfalse, _⟩
      · exact hP
      · exact absurd hfalse (by simp)
    · intro hP
      exact (l28_tcp_hw_ok_fate_iff bh true).mpr (Or.inl ⟨rfl, hP⟩)

/-- RFC-0210 P2.2: the two conjuncts FUSE into one ctor pass —
    the bound left-check feeds the hw-check and the composed
    removal verdict is ok exactly when BOTH fates hold (a failed
    left fate can never carry a preserved high-water verdict). -/
theorem l28_removal_protocol_fused :
    ∀ (bl bh : Bool),
      (Aeneas.Std.bind (l28_tcp_left_ok bl)
          (fun l => Aeneas.Std.bind (l28_tcp_hw_ok bh)
            (fun h => ok (l && h))) = ok true) ↔
        (bl = true ∧ bh = true) := by
  intro bl bh
  unfold l28_tcp_left_ok l28_tcp_hw_ok
  cases bl <;> cases bh <;> simp [Aeneas.Std.bind]
