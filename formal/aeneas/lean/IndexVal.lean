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
