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

/-- Cut tag is the identity: the packed length is kept. -/
theorem pack_cut_tag_fate_iff :
    ∀ (len r : U32),
      (pack_cut_tag len = ok r) ↔ (r = len) := by
  intro len r
  unfold pack_cut_tag
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl
