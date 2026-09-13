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
