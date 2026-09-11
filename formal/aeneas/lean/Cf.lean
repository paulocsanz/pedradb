-- Theorems over Aeneas extract of cf_kernel.rs (RFC-0150 P0).
-- Charon --start-from catalog entries; cf_encode_effective / decode_cf_key
-- patched (lifetime/'a str bottoms) in aeneas_cf.sh.
import Aeneas
import CfKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_cf_kernel

/-- AS-IS dente: every key is in-family. -/
theorem key_in_cf_family_as_is_dente (k f) :
    key_in_cf_family_as_is k f = ok true := by
  unfold key_in_cf_family_as_is
  rfl

/-- Catalog entry: extracted effective prefix is eq-default then empty-or-cf. -/
theorem cf_encode_effective_is_if (cf default_raw) :
    cf_encode_effective cf default_raw = (
      do
        let b ← Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default")
        if b && default_raw then ok (toStr "") else ok cf
    ) := by
  unfold cf_encode_effective
  rfl

/-- Catalog entry: no bounds ⇒ mixed/empty tag. -/
theorem infer_sst_cf_none_none :
    infer_sst_cf none none = alloc.string.String.new := by
  unfold infer_sst_cf
  rfl

/-- AS-IS dente: compact rewrites every SST. -/
theorem compact_rewrites_sst_cf_as_is_dente (s f) :
    compact_rewrites_sst_cf_as_is s f = ok true := by
  unfold compact_rewrites_sst_cf_as_is
  rfl

/-- Catalog entry: the extract axiomatizes str-eq and `is_empty`; given they
compute as rustc does on `"default"`/`""`, eq-default + default_raw ⇒ decode
is identity. -/
theorem decode_cf_key_default_raw_is_identity
    (heq : Str.Insts.CoreCmpPartialEqStr.eq (toStr "default") (toStr "default") = ok true)
    (hempty : core.str.Str.is_empty (toStr "") = ok true)
    (s : Slice Std.U8) :
    decode_cf_key (toStr "default") s true = ok s := by
  unfold decode_cf_key
  simp [cf_encode_effective, heq, hempty]

/-- Catalog entry: same axiom boundary ⇒ encode copies the bare key. -/
theorem encode_cf_key_default_raw_is_key
    (heq : Str.Insts.CoreCmpPartialEqStr.eq (toStr "default") (toStr "default") = ok true)
    (hempty : core.str.Str.is_empty (toStr "") = ok true)
    (k : Slice Std.U8) :
    encode_cf_key (toStr "default") k true =
      alloc.slice.Slice.to_vec core.clone.CloneU8 k := by
  unfold encode_cf_key
  simp [cf_encode_effective, heq, hempty]

/-- Catalog entry: the effective column-family encoding is empty
    exactly when (the cf is "default" and default_raw strips it) or
    (the else-branch kept an already-empty cf) — any other cf passes
    through untouched (RFC-0150 P0). The Str equality is the Aeneas
    boundary: stated over its result, so fail/div of the comparison
    never fakes an empty encoding. -/
theorem cf_encode_effective_empty_iff_default_raw_else_identity :
    ∀ (cf : Str) (default_raw : Bool),
      (cf_encode_effective cf default_raw = ok (toStr ""))
        ↔ ((Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default") = ok true
            ∧ default_raw = true)
          ∨ (((Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default") = ok true
              ∧ default_raw = false)
              ∨ Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default") = ok false)
            ∧ cf = toStr "")) := by
  intro cf default_raw
  unfold cf_encode_effective
  cases he : Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default") with
  | ok b =>
    cases b <;> cases default_raw <;> simp
  | fail e =>
    constructor
    · intro h
      simp at h
    · rintro (⟨h1, _⟩ | ⟨(⟨h1, _⟩ | h1), _⟩) <;>
      exact absurd h1 (by simp)
  | div =>
    constructor
    · intro h
      simp at h
    · rintro (⟨h1, _⟩ | ⟨(⟨h1, _⟩ | h1), _⟩) <;>
      exact absurd h1 (by simp)

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

/-- Catalog entry: a compact of `family` rewrites an SST tagged `sst_cf`
    exactly along the encode-representative route — an empty (mixed /
    legacy) tag is never rewritten (the value is false); a non-empty tag
    is decided by the in-family test over the representative encoded key
    of that tag, with every monadic step of the route ok (RFC-0150 P0). -/
theorem compact_rewrites_sst_cf_ok_iff_empty_tag_never_or_representative_in_family :
    ∀ (sst_cf : Str) (family : Str) (v : Bool),
    (compact_rewrites_sst_cf sst_cf family = ok v) ↔
      ((∃ b, core.str.Str.is_empty sst_cf = ok b ∧ b = true ∧ v = false) ∨
       (∃ b s enc, core.str.Str.is_empty sst_cf = ok b ∧ ¬(b = true) ∧
          lift (Array.to_slice (Std.Array.empty Std.U8)) = ok s ∧
          encode_cf_key sst_cf s false = ok enc ∧
          key_in_cf_family (alloc.vec.Vec.deref enc) family = ok v)) := by
  intro sst_cf family v
  unfold compact_rewrites_sst_cf
  constructor
  · intro hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      injection hval with hv
      exact Or.inl ⟨b, hb, hbt, hv.symm⟩
    · next hbt =>
      obtain ⟨s, hlift, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨enc, henc, hval⟩ := bind_ok_inv _ _ _ hval
      exact Or.inr ⟨b, s, enc, hb, hbt, hlift, henc, hval⟩
  · rintro (⟨b, hb, hbt, hv⟩ | ⟨b, s, enc, hb, hbf, hlift, henc, hkin⟩)
    · refine bind_intro b hb ?_
      rw [if_pos hbt, hv]
    · refine bind_intro b hb ?_
      rw [if_neg hbf]
      refine bind_intro s hlift ?_
      refine bind_intro enc henc ?_
      exact hkin

/-- Catalog entry: decoding an encoded key is the identity exactly when
    the effective cf encoding is empty; otherwise the cf\0 prefix is
    stripped by slicing from len+1 — and when even that prefix does not
    fit the encoded buffer, the decode yields the empty slice, never a
    partial or shifted view (RFC-0150 P0). -/
theorem decode_cf_key_ok_iff_identity_or_stripped_past_prefix :
    ∀ (cf : Str) (encoded : Slice Std.U8) (default_raw : Bool) (v : Slice Std.U8),
    (decode_cf_key cf encoded default_raw = ok v) ↔
      ((∃ eff b, cf_encode_effective cf default_raw = ok eff ∧
          core.str.Str.is_empty eff = ok b ∧ b = true ∧ encoded = v) ∨
       (∃ eff b i i1, cf_encode_effective cf default_raw = ok eff ∧
          core.str.Str.is_empty eff = ok b ∧ ¬(b = true) ∧
          core.str.Str.len eff = ok i ∧
          i + 1#usize = ok i1 ∧
          ((i1 > Slice.len encoded ∧
              lift (Array.to_slice (Std.Array.empty Std.U8)) = ok v) ∨
           (¬(i1 > Slice.len encoded) ∧
              core.slice.index.Slice.index
                (core.slice.index.SliceIndexRangeFromUsizeSlice Std.U8)
                encoded { start := i1 } = ok v)))) := by
  intro cf encoded default_raw v
  unfold decode_cf_key
  constructor
  · intro hval
    obtain ⟨eff, heff, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      injection hval with hv
      exact Or.inl ⟨eff, b, heff, hb, hbt, hv⟩
    · next hbt =>
      obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
      simp only [] at hval
      split at hval
      · next hgt =>
        exact Or.inr ⟨eff, b, i, i1, heff, hb, hbt, hi, hi1, Or.inl ⟨hgt, hval⟩⟩
      · next hgt =>
        exact Or.inr ⟨eff, b, i, i1, heff, hb, hbt, hi, hi1, Or.inr ⟨hgt, hval⟩⟩
  · rintro (⟨eff, b, heff, hb, hbt, hv⟩ |
      ⟨eff, b, i, i1, heff, hb, hbt, hi, hi1, (⟨hgt, hlift⟩ | ⟨hgt, hidx⟩)⟩)
    · refine bind_intro eff heff ?_
      refine bind_intro b hb ?_
      rw [if_pos hbt, hv]
    · refine bind_intro eff heff ?_
      refine bind_intro b hb ?_
      rw [if_neg hbt]
      refine bind_intro i hi ?_
      refine bind_intro i1 hi1 ?_
      simp only []
      rw [if_pos hgt]
      exact hlift
    · refine bind_intro eff heff ?_
      refine bind_intro b hb ?_
      rw [if_neg hbt]
      refine bind_intro i hi ?_
      refine bind_intro i1 hi1 ?_
      simp only []
      rw [if_neg hgt]
      exact hidx
