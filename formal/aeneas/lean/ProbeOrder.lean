-- Theorems over Aeneas extract of probe_order_kernel.rs (RFC-0164).
-- Charon --start-from first_probe_on_equal_lo (walk is Iterator-refused).
import Aeneas
import ProbeOrderKernel
open Aeneas.Std Result
open pedra_aeneas_probe_order_kernel

/-- Catalog entry: equal-lo tie probes the newer table first. -/
theorem first_probe_on_equal_lo_newer :
    first_probe_on_equal_lo (1#usize) (0#usize) = ok (1#usize) := by
  unfold first_probe_on_equal_lo
  rfl

/-- AS-IS dente: equal-lo tie probes the older table first. -/
theorem first_probe_on_equal_lo_as_is_dente :
    first_probe_on_equal_lo_as_is (1#usize) (0#usize) = ok (0#usize) := by
  unfold first_probe_on_equal_lo_as_is
  rfl

/-- Packed covering: `hi` past the array is not `>= key`. -/
theorem covering_hi_ge_oob :
    covering_hi_ge ⟨[], by native_decide⟩ (0#usize) ⟨[], by native_decide⟩
      = ok false := by
  unfold covering_hi_ge
  rfl

/-- Catalog entry is a Lean `def` (index walk). Unfolds to the extracted loop. -/
theorem probe_order_covering_is_loop (nf by_lo pe his key) :
    probe_order_covering nf by_lo pe his key
      = probe_order_covering_loop nf by_lo pe his key
          (alloc.vec.Vec.with_capacity Usize (Slice.len nf)) 0#usize := by
  unfold probe_order_covering
  rfl

/-- AS-IS dente: oldest-first is the reverse-index loop. -/
theorem probe_order_covering_as_is_is_loop (nf by_lo pe his key) :
    probe_order_covering_as_is nf by_lo pe his key
      = probe_order_covering_as_is_loop nf by_lo pe his key
          (alloc.vec.Vec.with_capacity Usize (Slice.len nf)) 0#usize := by
  unfold probe_order_covering_as_is
  rfl


/-- Any ok-valued Result bind forces the bound term to be ok. -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- An ok chain reassembles into an ok bind. -/
private theorem bind_intro {α β} {x : Result α} {f : α → Result β} {v : β}
    (a : α) (hx : x = ok a) (h : f a = ok v) : Aeneas.Std.bind x f = ok v := by
  rw [hx]
  exact h

/-- RFC-0218 P2.2 (átomo `catalog:probe_order`, entrada
    `first_probe_on_equal_lo`): empate de lo probeia EXATAMENTE o mais
    novo — o resultado é o índice `newer` citado. O AS-IS devolve o mais
    velho (resurrect no empate — dente plantado). -/
theorem probe_order_fate_iff :
    ∀ (newer older : Usize) (v : Usize),
      (first_probe_on_equal_lo newer older = ok v) ↔ (v = newer) := by
  intro newer older v
  constructor
  · intro hval
    unfold first_probe_on_equal_lo at hval
    injection hval with hv
    exact hv.symm
  · rintro rfl
    unfold first_probe_on_equal_lo
    rfl

/-- RFC-0218 P2.2 (átomo `catalog:probe_order_covering`, entrada
    `probe_order_covering`): a lista de candidatos é EXATAMENTE o loop
    citado — newest_first com capacidade len newest_first, do zero, com
    o porte covering_pos e o gate covering_hi_ge decidindo push/skip. -/
theorem probe_order_covering_fate_iff :
    ∀ (newest_first : Slice Usize) (by_lo : Slice Usize)
      (prefix_end : Usize) (his : Slice (Slice U8)) (key : Slice U8)
      (out : alloc.vec.Vec Usize),
      (probe_order_covering newest_first by_lo prefix_end his key = ok out) ↔
        (probe_order_covering_loop newest_first by_lo prefix_end his key
           (alloc.vec.Vec.with_capacity Usize (Slice.len newest_first))
             0#usize = ok out) := by
  intro newest_first by_lo prefix_end his key out
  unfold probe_order_covering
  exact Iff.rfl

