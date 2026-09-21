-- Theorems over Aeneas extract of sst/magic_kernel.rs (RFC-0186 P2.2 /
-- RFC-0218 P0.4 9/9). Extract by scripts/aeneas_magic.sh (shim re-exposes
-- table.rs SST_MAGIC; the script fails if the copy drifts).
import Aeneas
import MagicKernel
open Aeneas.Std Result
open pedra_aeneas_magic_kernel

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

/-- RFC-0218 P0.4 9/9 (atom `catalog:sst_magic`, entrada
    `sst_magic_is_pedra`): a admissão de mágica é EXATAMENTE a cadeia
    citada — sem os 8 bytes de prefixo (len do header < len da
    mágica) recusa (false); com prefixo possível, admite sse o
    slice-eq do prefixo contra PEDRSST\\0 casa (o compare final
    decide). O AS-IS admite qualquer header (a mentira drop-in em
    disco — tooth plantado). -/
theorem sst_magic_is_pedra_fate_iff :
    ∀ (header : Slice U8) (v : Bool),
      (magic_kernel.sst_magic_is_pedra header = ok v) ↔
        (∃ s, lift (Array.to_slice table.SST_MAGIC) = ok s ∧
          ((¬(Slice.len header ≥ Slice.len s) ∧ v = false) ∨
            (Slice.len header ≥ Slice.len s ∧
              ∃ s1, lift (Array.to_slice table.SST_MAGIC) = ok s1 ∧
              ∃ s2, core.slice.index.Slice.index
                  (core.slice.index.SliceIndexRangeToUsizeSlice U8) header
                  { «end» := Slice.len s1 } = ok s2 ∧
              ∃ s3, core.array.Array.index
                  (core.ops.index.IndexSlice
                    (core.ops.range.RangeFull.Insts.CoreSliceIndexSliceIndexSliceSlice U8))
                  table.SST_MAGIC () = ok s3 ∧
                core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 s2 s3 = ok v))) := by
  intro header v
  constructor
  · intro hval
    unfold magic_kernel.sst_magic_is_pedra at hval
    obtain ⟨s, hs, hval⟩ := bind_ok_inv _ _ _ hval
    simp only [] at hval
    split at hval
    · next hc =>
      obtain ⟨s1, hs1, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨s2, hs2, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨s3, hs3, hval⟩ := bind_ok_inv _ _ _ hval
      exact ⟨s, hs, Or.inr ⟨hc, s1, hs1, s2, hs2, s3, hs3, hval⟩⟩
    · next hc =>
      injection hval with hv
      exact ⟨s, hs, Or.inl ⟨hc, hv.symm⟩⟩
  · rintro ⟨s, hs, (⟨hc, hv⟩ | ⟨hc, s1, hs1, s2, hs2, s3, hs3, hv⟩)⟩
    · subst hv
      unfold magic_kernel.sst_magic_is_pedra
      exact bind_intro s hs (by
        show (if Slice.len header ≥ Slice.len s then _ else ok false) = ok false
        rw [if_neg hc])
    · unfold magic_kernel.sst_magic_is_pedra
      exact bind_intro s hs (by
        show (if Slice.len header ≥ Slice.len s then _ else ok false) = ok v
        rw [if_pos hc]
        exact bind_intro s1 hs1
          (bind_intro s2 hs2 (bind_intro s3 hs3 hv)))
