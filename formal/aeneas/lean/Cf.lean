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

/-- Catalog entry: encoding a key yields the bare key exactly when the
    effective cf encoding is empty; otherwise the output is built by the
    capacity-planned chain — effective bytes, one 0 separator, then the
    key — with every step of the chain ok (RFC-0150 P0). -/
theorem encode_cf_key_ok_iff_bare_key_or_prefixed_vec :
    ∀ (cf : Str) (key : Slice Std.U8) (default_raw : Bool)
      (v : alloc.vec.Vec Std.U8),
    (encode_cf_key cf key default_raw = ok v) ↔
      ((∃ eff b, cf_encode_effective cf default_raw = ok eff ∧
          core.str.Str.is_empty eff = ok b ∧ b = true ∧
          alloc.slice.Slice.to_vec core.clone.CloneU8 key = ok v) ∨
       (∃ eff b i i1 i3 s enc1 enc2,
          cf_encode_effective cf default_raw = ok eff ∧
          core.str.Str.is_empty eff = ok b ∧ ¬(b = true) ∧
          core.str.Str.len eff = ok i ∧
          i + 1#usize = ok i1 ∧
          i1 + Slice.len key = ok i3 ∧
          core.str.Str.as_bytes eff = ok s ∧
          alloc.vec.Vec.extend_from_slice core.clone.CloneU8
            (alloc.vec.Vec.with_capacity Std.U8 i3) s = ok enc1 ∧
          alloc.vec.Vec.push enc1 0#u8 = ok enc2 ∧
          alloc.vec.Vec.extend_from_slice core.clone.CloneU8 enc2 key
            = ok v)) := by
  intro cf key default_raw v
  unfold encode_cf_key
  constructor
  · intro hval
    obtain ⟨eff, heff, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      exact Or.inl ⟨eff, b, heff, hb, hbt, hval⟩
    · next hbt =>
      obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨i3, hi3, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨s, hs, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨enc1, henc1, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨enc2, henc2, hval⟩ := bind_ok_inv _ _ _ hval
      exact Or.inr ⟨eff, b, i, i1, i3, s, enc1, enc2, heff, hb, hbt,
        hi, hi1, hi3, hs, henc1, henc2, hval⟩
  · rintro (⟨eff, b, heff, hb, hbt, hv⟩ |
      ⟨eff, b, i, i1, i3, s, enc1, enc2, heff, hb, hbt, hi, hi1, hi3,
        hs, henc1, henc2, hv⟩)
    · refine bind_intro eff heff ?_
      refine bind_intro b hb ?_
      rw [if_pos hbt]
      exact hv
    · refine bind_intro eff heff ?_
      refine bind_intro b hb ?_
      rw [if_neg hbt]
      refine bind_intro i hi ?_
      refine bind_intro i1 hi1 ?_
      refine bind_intro i3 hi3 ?_
      refine bind_intro s hs ?_
      refine bind_intro enc1 henc1 ?_
      refine bind_intro enc2 henc2 ?_
      exact hv

/-- Catalog entry: an SST is tagged with the family of its bounds
    exactly when both bounds share one family; one-sided bounds take
    their sole family; no bounds yield the empty (mixed/legacy) tag —
    never a tag that misrepresents mixed contents (RFC-0150 P0). -/
theorem infer_sst_cf_ok_iff_shared_family_or_empty :
    ∀ (smallest largest : Option (Slice Std.U8)) (v : String),
    (infer_sst_cf smallest largest = ok v) ↔
      ((smallest = none ∧ largest = none ∧
          alloc.string.String.new = ok v) ∨
       (∃ s, smallest = none ∧ largest = some s ∧
          cf_family_of s = ok v) ∨
       (∃ s, smallest = some s ∧ largest = none ∧
          cf_family_of s = ok v) ∨
       (∃ s l a b b1, smallest = some s ∧ largest = some l ∧
          cf_family_of s = ok a ∧ cf_family_of l = ok b ∧
          alloc.string.String.Insts.CoreCmpPartialEqString.eq a b = ok b1 ∧
          ((b1 = true ∧ a = v) ∨
            (¬(b1 = true) ∧ alloc.string.String.new = ok v)))) := by
  intro smallest largest v
  unfold infer_sst_cf
  constructor
  · intro hval
    cases smallest with
    | none =>
      cases largest with
      | none => exact Or.inl ⟨rfl, rfl, hval⟩
      | some s => exact Or.inr (Or.inl ⟨s, rfl, rfl, hval⟩)
    | some s =>
      cases largest with
      | none => exact Or.inr (Or.inr (Or.inl ⟨s, rfl, rfl, hval⟩))
      | some l =>
        obtain ⟨a, ha, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
        split at hval
        · next hbt =>
          injection hval with hv
          exact Or.inr (Or.inr (Or.inr
            ⟨s, l, a, b, b1, rfl, rfl, ha, hb, hb1, Or.inl ⟨hbt, hv⟩⟩))
        · next hbt =>
          exact Or.inr (Or.inr (Or.inr
            ⟨s, l, a, b, b1, rfl, rfl, ha, hb, hb1, Or.inr ⟨hbt, hval⟩⟩))
  · rintro (⟨h1, h2, h3⟩ | ⟨s, h1, h2, h3⟩ | ⟨s, h1, h2, h3⟩ |
      ⟨s, l, a, b, b1, h1, h2, h3, h4, h5,
        (⟨hbt, hva⟩ | ⟨hbt, hnew⟩)⟩)
    · subst h1; subst h2; exact h3
    · subst h2; subst h1; exact h3
    · subst h2; subst h1; exact h3
    · subst h2; subst h1
      refine bind_intro a h3 ?_
      refine bind_intro b h4 ?_
      refine bind_intro b1 h5 ?_
      rw [if_pos hbt, hva]
    · subst h2; subst h1
      refine bind_intro a h3 ?_
      refine bind_intro b h4 ?_
      refine bind_intro b1 h5 ?_
      rw [if_neg hbt]
      exact hnew

/-- RFC-0213 P1.1 (storage cadence, atom `catalog:cf_family`): a key
    belongs to a column family EXACTLY along the extracted route —
    for the default family, no separator or a leading separator is
    in-family and any other prefix must equal the "default" bytes;
    for a named family, the key must strictly overhang the family
    bytes, start with them, and carry the 0 separator right after
    (fate forall over the extracted body, RFC-0170 P2.4). The AS-IS
    mutant answers in-family for every key (the lie the DST plant
    `key_in_cf_family_on_live_scan_is_not_ok` refutes). -/
theorem cf_family_fate_iff :
    ∀ (user_key : Slice Std.U8) (family : Str) (v : Bool),
    (key_in_cf_family user_key family = ok v) ↔
      ((∃ b i o u,
          Str.Insts.CoreCmpPartialEqStr.eq family (toStr "default") = ok b ∧
          b = true ∧
          core.slice.Slice.iter user_key = ok i ∧
          core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.position
            key_in_cf_family.closure.Insts.CoreOpsFunctionFnMutTupleSharedU8Bool i ()
            = ok (o, u) ∧
          ((o = none ∧ v = true) ∨
           (∃ i1, o = some i1 ∧
              ((i1.val = 0 ∧ v = true) ∨
               (∃ s, ¬(i1.val = 0) ∧
                  core.slice.index.Slice.index
                    (core.slice.index.SliceIndexRangeToUsizeSlice Std.U8) user_key
                    { «end» := i1 } = ok s ∧
                  Slice.Insts.CoreCmpPartialEqArray.eq core.cmp.PartialEqU8 s
                    (Array.make 7#usize
                      [100#u8, 101#u8, 102#u8, 97#u8, 117#u8, 108#u8, 116#u8])
                    = ok v)))))
        ∨ (∃ b p,
          Str.Insts.CoreCmpPartialEqStr.eq family (toStr "default") = ok b ∧
          ¬(b = true) ∧
          core.str.Str.as_bytes family = ok p ∧
          ((Slice.len user_key > Slice.len p ∧
            ((∃ b1, core.slice.Slice.starts_with core.cmp.PartialEqU8 user_key p
                = ok b1 ∧
                ((b1 = true ∧
                  (∃ i3, Slice.index_usize user_key (Slice.len p) = ok i3 ∧
                    v = decide (i3 = 0#u8)))
                  ∨ (¬(b1 = true) ∧ v = false))))
            ∨ (¬(Slice.len user_key > Slice.len p) ∧ v = false))))) := by
  intro user_key family v
  unfold key_in_cf_family
  constructor
  · intro hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨pair, hpair, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨o, u⟩ := pair
      refine Or.inl ⟨b, i, o, u, hb, hbt, hi, hpair, ?_⟩
      cases o with
      | none =>
        refine Or.inl ⟨rfl, ?_⟩
        injection hval with hv
        exact hv.symm
      | some i1 =>
        refine Or.inr ⟨i1, rfl, ?_⟩
        conv at hval => lhs; whnf
        split at hval
        · next hz =>
          exact Or.inl ⟨hz, by injection hval with hv; exact hv.symm⟩
        · next hz =>
          obtain ⟨s, hs, hval⟩ := bind_ok_inv _ _ _ hval
          exact Or.inr ⟨s, hz, hs, hval⟩
    · next hbt =>
      obtain ⟨p, hp, hval⟩ := bind_ok_inv _ _ _ hval
      refine Or.inr ⟨b, p, hb, hbt, hp, ?_⟩
      simp only [] at hval
      split at hval
      · next hgt =>
        obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
        refine Or.inl ⟨hgt, b1, hb1, ?_⟩
        split at hval
        · next hbt1 =>
          obtain ⟨i3, hi3, hval⟩ := bind_ok_inv _ _ _ hval
          exact Or.inl ⟨hbt1, i3, hi3, by injection hval with hv; exact hv.symm⟩
        · next hbt1 =>
          exact Or.inr ⟨hbt1, by injection hval with hv; exact hv.symm⟩
      · next hgt =>
        exact Or.inr ⟨hgt, by injection hval with hv; exact hv.symm⟩
  · rintro (⟨b, i, o, u, hb, hbt, hi, hpos, hlast⟩ |
      ⟨b, p, hb, hbt, hp, hlast⟩)
    · refine bind_intro b hb ?_
      rw [if_pos hbt]
      refine bind_intro i hi ?_
      refine bind_intro (o, u) hpos ?_
      cases o with
      | none =>
        rcases hlast with ⟨-, hv⟩ | ⟨i1, hbad, -⟩
        · exact congrArg ok hv.symm
        · exact absurd hbad (by simp)
      | some i1 =>
        rcases hlast with ⟨hbad, -⟩ | ⟨i1', heqo, hzvh⟩
        · exact absurd hbad (by simp)
        · injection heqo with e
          subst e
          conv => lhs; whnf
          rcases hzvh with ⟨hz, hv⟩ | ⟨s, hz, hidx, heq⟩
          · rw [hz]
            exact congrArg ok hv.symm
          · split
            · next hz' => exact absurd hz' hz
            · refine bind_intro s hidx ?_
              exact heq
    · refine bind_intro b hb ?_
      rw [if_neg hbt]
      refine bind_intro p hp ?_
      simp only []
      rcases hlast with ⟨hgt, hrest⟩ | ⟨hgt, hv⟩
      · rw [if_pos hgt]
        rcases hrest with ⟨b1, hb1, hzvh⟩
        refine bind_intro b1 hb1 ?_
        rcases hzvh with ⟨hbt1, i3, hi3, hv⟩ | ⟨hbt1, hv⟩
        · rw [if_pos hbt1]
          refine bind_intro i3 hi3 ?_
          rw [hv]
        · rw [if_neg hbt1]
          exact congrArg ok hv.symm
      · rw [if_neg hgt]
        exact congrArg ok hv.symm
