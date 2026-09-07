-- Theorems over Aeneas extract of index_val_kernel.rs
import Aeneas
import IndexValKernel
open Aeneas.Std Result
open pedra_aeneas_index_val_kernel

/-- Catalog entry `value_len_tag` is identity on the extracted term. -/
theorem value_len_tag_identity :
    value_len_tag 4#u32 = ok (4#u32) := by
  unfold value_len_tag
  rfl

/-- AS-IS F80: length tag dropped. -/
theorem value_len_tag_as_is_dente :
    value_len_tag_as_is 4#u32 = ok (0#u32) := by
  unfold value_len_tag_as_is
  rfl
