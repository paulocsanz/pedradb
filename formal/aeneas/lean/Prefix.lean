-- Theorems over the Aeneas extract of production prefix.rs (RFC-0170 P0.3).
import Aeneas
import PrefixKernel
open Aeneas.Std Result ControlFlow
open pedra_aeneas_prefix_kernel

/-- The extracted loop body on an empty vec is `done none` (all-0xff / empty prefix). -/
theorem prefix_exclusive_end_matches_spec
    (e : alloc.vec.Vec U8)
    (h : alloc.vec.Vec.len e = 0#usize) :
    prefix_exclusive_end_loop.body e = ok (done none) := by
  unfold prefix_exclusive_end_loop.body
  simp [h]

/-- Nonempty prefix: last byte `< 0xff` bumps, else pop and continue. Dual-unfold. -/
theorem prefix_exclusive_end_loop_body_nonempty
    (e : alloc.vec.Vec U8)
    (h : alloc.vec.Vec.len e > 0#usize) :
    prefix_exclusive_end_loop.body e =
      (do
        let i1 := alloc.vec.Vec.len e
        let i2 ← i1 - 1#usize
        let i3 ←
          alloc.vec.Vec.index (core.slice.index.SliceIndexUsizeSlice U8) e i2
        if i3 < 255#u8 then
          (do
            let (i4, index_mut_back) ←
              alloc.vec.Vec.index_mut (core.slice.index.SliceIndexUsizeSlice U8)
                e i2
            let i5 ← i4 + 1#u8
            let e1 := index_mut_back i5
            ok (done (some e1)))
        else
          (do
            let (_, e1) ← alloc.vec.Vec.pop Global e
            ok (cont e1))) := by
  unfold prefix_exclusive_end_loop.body
  simp [h]

/-- Extracted production fn is to_vec then the loop. -/
theorem prefix_exclusive_end_def (p : Slice U8) :
    prefix_exclusive_end p =
      (do
        let e ← alloc.slice.Slice.to_vec core.clone.CloneU8 p
        prefix_exclusive_end_loop e) := rfl

/-- F57/F58 AS-IS tooth: extracted mutant is to_vec then push 255 (`prefix || 0xff`). -/
theorem prefix_exclusive_end_as_is_tooth (p : Slice U8) :
    prefix_exclusive_end_as_is p =
      (do
        let e ← alloc.slice.Slice.to_vec core.clone.CloneU8 p
        let e1 ← alloc.vec.Vec.push e 255#u8
        ok (some e1)) := rfl

/-- Catalog entry: unbounded prefix end is `starts_with` (rustc `&[u8]`). Dual-unfold. -/
theorem key_in_prefix_range_unbounded_end (user pref) :
    key_in_prefix_range user pref none =
      (do
        let b ← core.slice.Slice.starts_with core.cmp.PartialEqU8 user pref
        if b then ok true else ok false) := by
  unfold key_in_prefix_range
  rfl

/-- Exclusive end after a prefix hit is slice `<` (F57/F58). Dual-unfold. -/
theorem key_in_prefix_range_exclusive (user pref e) :
    key_in_prefix_range user pref (some e) =
      (do
        let b ← core.slice.Slice.starts_with core.cmp.PartialEqU8 user pref
        if b then
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        else ok false) := by
  unfold key_in_prefix_range
  rfl

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

/-- RFC-0218 P1.2 4/11 (atom `catalog:prefix`, entrada
    `prefix_exclusive_end`): o fim exclusivo é EXATAMENTE o
    encaminhamento citado — o prefixo vira Vec e o loop citado decide
    (incrementa o último byte ou some). O AS-IS empurra 255 (fim
    errado engole chaves — tooth plantado). -/
theorem prefix_exclusive_end_fate_iff :
    ∀ (prefix1 : Slice U8) (r : Option (alloc.vec.Vec U8)),
      (prefix_exclusive_end prefix1 = ok r) ↔
      (∃ e, alloc.slice.Slice.to_vec core.clone.CloneU8 prefix1 = ok e ∧
            prefix_exclusive_end_loop e = ok r) := by
  intro prefix1 r
  constructor
  · intro hval
    unfold prefix_exclusive_end at hval
    exact bind_ok_inv _ _ _ hval
  · rintro ⟨e, he, hs⟩
    unfold prefix_exclusive_end
    exact bind_intro e he hs
