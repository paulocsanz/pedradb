-- Theorems over Aeneas extract of pack_kernel.rs
import Aeneas
import PackKernel
open Aeneas.Std Result
open pedra_aeneas_pack_kernel

theorem pack_cut_tag_identity :
    pack_cut_tag 7#u32 = ok (7#u32) := by
  unfold pack_cut_tag
  rfl

theorem pack_cut_tag_as_is_dente :
    pack_cut_tag_as_is 7#u32 = ok (0#u32) := by
  unfold pack_cut_tag_as_is
  rfl
