-- Theorems over Aeneas extract of index_val_kernel.rs
-- (RFC-0002 P26 / F78 / F80). Payment is the linked rustc bodies; the
-- former cfg(verus_keep_ghost) stand-in was deleted. Fail-closed: no holes.
import Aeneas
import IndexValKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_index_val_kernel

/-- Catalog entry `value_len_tag` is identity on the extracted term. -/
theorem value_len_tag_identity :
    value_len_tag 4#u32 = ok (4#u32) := by
  unfold value_len_tag
  rfl

/-- F80 teeth: the FIXED length tag is the length — always, general. -/
theorem value_len_tag_always_the_length :
    ∀ len : Std.U32, value_len_tag len = ok len := by
  intro len
  rfl

/-- AS-IS F80: length tag dropped. -/
theorem value_len_tag_as_is_dente :
    value_len_tag_as_is 4#u32 = ok (0#u32) := by
  unfold value_len_tag_as_is
  rfl

/-- AS-IS F80 dente, general: every length maps to the same tag 0 —
    `red` and `red\0foo` collide in the child range. -/
theorem value_len_tag_as_is_always_zero :
    ∀ len : Std.U32, value_len_tag_as_is len = ok 0#u32 := by
  intro len
  rfl

/-- RFC-0218 P1.2 1/11 (átomo `catalog:len_tag`, entrada
    `value_len_tag`): a etiqueta de comprimento é EXATAMENTE o lift
    citado `len` (identidade — por isso injetiva em len). O AS-IS é
    a constante 0 (etiqueta colapsada — dente plantado). -/
theorem value_len_tag_fate_iff :
    ∀ (len : U32) (r : U32),
      (value_len_tag len = ok r) ↔ (r = len) := by
  intro len r
  constructor
  · intro hval
    unfold value_len_tag at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
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

/-- RFC-0218 P1.2 5/11 (átomo `catalog:exact_children`, entrada
    `exact_value_children`): os filhos exatos são EXATAMENTE a cadeia
    citada — o prefixo vira Vec, empurra 0#u8 (início) e 1#u8 (fim).
    O AS-IS re-usa o len-pref e vaza irmão NUL (dente plantado). -/
theorem exact_value_children_fate_iff :
    ∀ (prefix1 : Slice U8) (r : (alloc.vec.Vec U8) × (alloc.vec.Vec U8)),
      (exact_value_children prefix1 = ok r) ↔
      (∃ st s1 e, alloc.slice.Slice.to_vec core.clone.CloneU8 prefix1 = ok st ∧
        alloc.vec.Vec.push st 0#u8 = ok s1 ∧
        alloc.vec.Vec.push st 1#u8 = ok e ∧
        r = (s1, e)) := by
  intro prefix1 r
  constructor
  · intro hval
    unfold exact_value_children at hval
    obtain ⟨st, hst, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨s1, hs1, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨e, he, hval⟩ := bind_ok_inv _ _ _ hval
    injection hval with hv
    exact ⟨st, s1, e, hst, hs1, he, hv.symm⟩
  · rintro ⟨st, s1, e, hst, hs1, he, hv⟩
    subst hv
    unfold exact_value_children
    exact bind_intro st hst (bind_intro s1 hs1 (bind_intro e he rfl))
