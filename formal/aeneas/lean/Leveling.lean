-- Theorems over Aeneas extract of leveling.rs (leveled compaction).
-- RUSTFLAGS=--cfg test so as_is mutants are visible; pick Iterator holes
-- patched to index loops in aeneas_leveling.sh.
import Aeneas
import LevelingKernel
open Aeneas.Std Result
open pedra_aeneas_leveling_kernel

/-- Catalog entry: L0 has no size target. -/
theorem level_target_bytes_l0 (t) :
    level_target_bytes (0#u32) t = ok (0#u64) := by
  unfold level_target_bytes
  rfl

/-- AS-IS tooth: L0 target is still zero. -/
theorem level_target_bytes_as_is_l0 (t) :
    level_target_bytes_as_is (0#u32) t = ok (0#u64) := by
  unfold level_target_bytes_as_is
  rfl

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

/-- Catalog entry: the leveled size target of a level computes to a
    value exactly along one of two dispositions — level 0 has no
    target (the value is 0), or the level is non-zero and every step of
    the saturating chain level−1, min 18, FANOUT^·, l1_target·· is ok,
    the final saturating_mul producing the value (F-leveling-sweep). -/
theorem level_target_bytes_ok_iff_zero_or_fanout_chain :
    ∀ (level : U32) (l1_target : U64) (v : U64),
    (level_target_bytes level l1_target = ok v) ↔
      ((level = 0#u32 ∧ v = 0#u64) ∨
        (¬(level = 0#u32) ∧ ∃ i e f,
          level - 1#u32 = ok i ∧
          core.cmp.Ord.min.trait_default core.cmp.OrdU32 i 18#u32 = ok e ∧
          core.num.U64.saturating_pow LEVEL_FANOUT e = ok f ∧
          core.num.U64.saturating_mul l1_target f = ok v)) := by
  intro level l1_target v
  unfold level_target_bytes
  split
  · next h =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨h, hv.symm⟩
    · rintro (⟨h0, hv⟩ | ⟨hne, i, e, f, hsub, hmin, hpow, hmul⟩)
      · rw [hv]
      · exact absurd h hne
  · next h =>
    constructor
    · intro hval
      obtain ⟨i, hsub, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨e, hmin, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨f, hpow, hval⟩ := bind_ok_inv _ _ _ hval
      exact Or.inr ⟨h, i, e, f, hsub, hmin, hpow, hval⟩
    · rintro (⟨h0, hv⟩ | ⟨hne, i, e, f, hsub, hmin, hpow, hmul⟩)
      · exact absurd h0 h
      · refine bind_intro i hsub ?_
        refine bind_intro e hmin ?_
        refine bind_intro f hpow ?_
        exact hmul

/-- RFC-0213 P1.2 (storage cadence, atom `catalog:leveling_pick`): the
    L0→L1 compaction job is picked EXACTLY along the extracted route —
    an empty L0 or a zero input cap yields no job; otherwise the cap
    `min(len l0, max_l0)` bounds the selection walk, the first file
    seeds the hull, and the sel/slice loops (loop atoms) decide the
    job with every monadic step ok (fate forall over the extracted
    body, RFC-0170 P2.4). The AS-IS mutant reabsorbs the whole L1
    (the lie the DST plant `pick_l0_to_l1_on_live_slice_is_not_ok`
    refutes). -/
theorem pick_l0_to_l1_fate_iff :
    ∀ (l0 l1 : Slice LevelFile) (max_l0 : Usize)
      (v : Option ((alloc.vec.Vec Usize) × (alloc.vec.Vec Usize))),
    (pick_l0_to_l1 l0 l1 max_l0 = ok v) ↔
      ((∃ b, core.slice.Slice.is_empty l0 = ok b ∧ b = true ∧ v = none) ∨
       (∃ b, core.slice.Slice.is_empty l0 = ok b ∧ ¬(b = true) ∧
          max_l0 = 0#usize ∧ v = none) ∨
       (∃ b f0 hull_lo hull_hi sel1 sel hull_lo1 hull_hi1 slice,
          core.slice.Slice.is_empty l0 = ok b ∧ ¬(b = true) ∧
          ¬(max_l0 = 0#usize) ∧
          Slice.index_usize l0 0#usize = ok f0 ∧
          alloc.vec.CloneVec.clone core.clone.CloneU8 f0.lo = ok hull_lo ∧
          alloc.vec.CloneVec.clone core.clone.CloneU8 f0.hi = ok hull_hi ∧
          alloc.vec.Vec.push (alloc.vec.Vec.new Usize) f0.idx = ok sel1 ∧
          pick_l0_sel_loop l0
            (if Slice.len l0 < max_l0 then Slice.len l0 else max_l0)
            sel1 hull_lo hull_hi 1#usize = ok (sel, hull_lo1, hull_hi1) ∧
          pick_l0_slice_loop l1 hull_lo1 hull_hi1
            (alloc.vec.Vec.new Usize) 0#usize = ok slice ∧
          v = some (sel, slice))) := by
  intro l0 l1 max_l0 v
  unfold pick_l0_to_l1
  constructor
  · intro hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      exact Or.inl ⟨b, hb, hbt, by injection hval with hv; exact hv.symm⟩
    · next hbt =>
      split at hval
      · next hzt =>
        exact Or.inr (Or.inl
          ⟨b, hb, hbt, hzt, by injection hval with hv; exact hv.symm⟩)
      · next hzt =>
        obtain ⟨f0, hf0, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨hull_lo, hlo, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨hull_hi, hhi, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨sel1, hsel1, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨triple, hloop, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨sel, hull_lo1, hull_hi1⟩ := triple
        obtain ⟨slice, hslice, hval⟩ := bind_ok_inv _ _ _ hval
        exact Or.inr (Or.inr ⟨b, f0, hull_lo, hull_hi, sel1, sel, hull_lo1,
          hull_hi1, slice, hb, hbt, hzt, hf0, hlo, hhi, hsel1, hloop,
          hslice, by injection hval with hv; exact hv.symm⟩)
  · rintro (⟨b, hb, hbt, hv⟩ |
      ⟨b, hb, hbt, hzt, hv⟩ |
      ⟨b, f0, hull_lo, hull_hi, sel1, sel, hull_lo1, hull_hi1, slice,
        hb, hbt, hzt, hf0, hlo, hhi, hsel1, hloop, hslice, hv⟩)
    · refine bind_intro b hb ?_
      rw [if_pos hbt, hv]
    · refine bind_intro b hb ?_
      rw [if_neg hbt, if_pos hzt, hv]
    · refine bind_intro b hb ?_
      rw [if_neg hbt, if_neg hzt]
      refine bind_intro f0 hf0 ?_
      refine bind_intro hull_lo hlo ?_
      refine bind_intro hull_hi hhi ?_
      refine bind_intro sel1 hsel1 ?_
      refine bind_intro (sel, hull_lo1, hull_hi1) hloop ?_
      refine bind_intro slice hslice ?_
      exact congrArg ok hv.symm

/-- RFC-0213 P1.2 (storage cadence, atom `catalog:leveling_pushdown`):
    one pushdown job from level n to n+1 is picked EXACTLY along the
    extracted route — an empty source level yields no job; a
    non-disjoint destination view is refused (the gate that stops the
    unbounded cascade); otherwise the oldest source file plus the
    destination files overlapping its bounds decide the job, with the
    slice loop and every monadic step ok (fate forall over the
    extracted body, RFC-0170 P2.4). The AS-IS mutant skips the
    disjoint gate (the lie the DST plant
    `pick_pushdown_on_live_pushdown_gate_is_not_ok` refutes). -/
theorem pick_pushdown_fate_iff :
    ∀ (src dst : Slice LevelFile)
      (v : Option (Usize × (alloc.vec.Vec Usize))),
    (pick_pushdown src dst = ok v) ↔
      ((∃ b, core.slice.Slice.is_empty src = ok b ∧ b = true ∧ v = none) ∨
       (∃ b d, core.slice.Slice.is_empty src = ok b ∧ ¬(b = true) ∧
          is_disjoint dst = ok d ∧
          ((d = true ∧
            (∃ source slice,
              Slice.index_usize src 0#usize = ok source ∧
              pick_l0_slice_loop dst source.lo source.hi
                (alloc.vec.Vec.new Usize) 0#usize = ok slice ∧
              v = some (source.idx, slice)))
           ∨ (¬(d = true) ∧ v = none)))) := by
  intro src dst v
  unfold pick_pushdown
  constructor
  · intro hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      exact Or.inl ⟨b, hb, hbt, by injection hval with hv; exact hv.symm⟩
    · next hbt =>
      obtain ⟨d, hd, hval⟩ := bind_ok_inv _ _ _ hval
      refine Or.inr ⟨b, d, hb, hbt, hd, ?_⟩
      split at hval
      · next hdt =>
        obtain ⟨source, hsource, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨slice, hslice, hval⟩ := bind_ok_inv _ _ _ hval
        exact Or.inl ⟨hdt, source, slice, hsource, hslice,
          by injection hval with hv; exact hv.symm⟩
      · next hdt =>
        exact Or.inr ⟨hdt, by injection hval with hv; exact hv.symm⟩
  · rintro (⟨b, hb, hbt, hv⟩ | ⟨b, d, hb, hbt, hd, hlast⟩)
    · refine bind_intro b hb ?_
      rw [if_pos hbt, hv]
    · refine bind_intro b hb ?_
      rw [if_neg hbt]
      refine bind_intro d hd ?_
      rcases hlast with ⟨hdt, source, slice, hsource, hslice, hv⟩ |
        ⟨hdt, hv⟩
      · rw [if_pos hdt]
        refine bind_intro source hsource ?_
        refine bind_intro slice hslice ?_
        exact congrArg ok hv.symm
      · rw [if_neg hdt]
        exact congrArg ok hv.symm

/-- RFC-0218 P1.2 7/11 (atom `catalog:leveling_disjoint`, entry
    `is_disjoint`): disjunction is EXACTLY the cited loop of the zero —
    `is_disjoint files` Is the outer_loop in 0#usize (in the pre, in the post).
    The AS-IS is the constante true (stack sobreposta accepts — tooth
    planted). -/
theorem is_disjoint_fate_iff :
    ∀ (files : Slice LevelFile) (v : Bool),
      (is_disjoint files = ok v) ↔
      (is_disjoint_outer_loop files 0#usize = ok v) := by
  intro files v
  constructor
  · intro hval
    unfold is_disjoint at hval
    exact hval
  · intro hs
    unfold is_disjoint
    exact hs

/-- RFC-0218 P1.2 8/11 (atom `catalog:leveling_overlaps`, entry
    `overlaps`): sobrepor the hull is EXACTLY the cited pair — the `lo`
    of the file does not pass of the hull_hi and the `hi` of the file not stays
    below of the hull_lo. The AS-IS ignora the limit superior (hull
    inflado — tooth planted). -/
theorem overlaps_fate_iff :
    ∀ (f : LevelFile) (hull_lo : Slice U8) (hull_hi : Slice U8) (v : Bool),
      (LevelFile.overlaps f hull_lo hull_hi = ok v) ↔
      (∃ s b, alloc.vec.Vec.as_slice Global f.lo = ok s ∧
        Shared1A.Insts.CoreCmpPartialOrdShared0B.le
          (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) s hull_hi = ok b ∧
        ((b = true ∧
          ∃ s1, alloc.vec.Vec.as_slice Global f.hi = ok s1 ∧
            Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
              (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) s1 hull_lo = ok v) ∨
         (b = false ∧ v = false))) := by
  intro f hull_lo hull_hi v
  constructor
  · intro hval
    unfold LevelFile.overlaps at hval
    obtain ⟨s, hs, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hc =>
      obtain ⟨s1, hs1, hval⟩ := bind_ok_inv _ _ _ hval
      exact ⟨s, b, hs, hb, Or.inl ⟨hc, s1, hs1, hval⟩⟩
    · next hc =>
      simp only [Bool.not_eq_true] at hc
      injection hval with hv
      exact ⟨s, b, hs, hb, Or.inr ⟨hc, hv.symm⟩⟩
  · rintro ⟨s, b, hs, hb, (⟨hc, s1, hs1, hge⟩ | ⟨hc, hv⟩)⟩
    · subst hc
      unfold LevelFile.overlaps
      exact bind_intro s hs (bind_intro true hb (bind_intro s1 hs1 hge))
    · subst hc
      subst hv
      unfold LevelFile.overlaps
      exact bind_intro s hs (bind_intro false hb rfl)

/-- RFC-0218 P1.2 9/11 (atom `catalog:leveling_total_bytes`, entry
    `total_bytes`): the total of the level is EXACTLY the adds cited — the
    iterator of the slice, the map that extracts `bytes` from each file, and the
    u64 sum. The AS-IS returns a COUNT of files (bytes swapped
    for items — tooth planted). -/
theorem total_bytes_fate_iff :
    ∀ (files : Slice LevelFile) (v : U64),
      (total_bytes files = ok v) ↔
      (∃ i m, core.slice.Slice.iter files = ok i ∧
        core.iter.traits.iterator.Iterator.map.default
          (core.iter.traits.iterator.IteratorSliceIter LevelFile)
          total_bytes.closure.Insts.CoreOpsFunctionFnMutTupleSharedLevelFileU64 i
          () = ok m ∧
        core.iter.traits.iterator.Iterator.sum.default
          (core.iter.adapters.map.Map.Insts.CoreIterTraitsIteratorIterator
            (core.iter.traits.iterator.IteratorSliceIter LevelFile)
            total_bytes.closure.Insts.CoreOpsFunctionFnMutTupleSharedLevelFileU64)
          U64.Insts.CoreIterTraitsAccumSumU64 m = ok v) := by
  intro files v
  constructor
  · intro hval
    unfold total_bytes at hval
    obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨m, hm, hval⟩ := bind_ok_inv _ _ _ hval
    exact ⟨i, m, hi, hm, hval⟩
  · rintro ⟨i, m, hi, hm, hv⟩
    unfold total_bytes
    exact bind_intro i hi (bind_intro m hm hv)

/-- RFC-0218 P1.2 11/11 (atom `catalog:leveled_enabled`, entry
    `leveled_enabled`): the leveled mode is EXACTLY the cited read
    of the PEDRA_LEVELED variable — absent/error turns on (true); present,
    turns on except when the trimmed value is "0". The AS-IS is the constant
    true (the env off-switch is swallowed — tooth planted). -/
theorem leveled_enabled_fate_iff :
    ∀ (b : Bool),
      (leveled_enabled = ok b) ↔
      (∃ r, std.env.var
          (Shared0T.Insts.CoreConvertAsRef
            Str.Insts.CoreConvertAsRefOsStr) (toStr "PEDRA_LEVELED") = ok r ∧
        ((∃ ov, r = core.result.Result.Ok ov ∧
          ∃ s s1, alloc.string.String.Insts.CoreOpsDerefDerefStr.deref ov = ok s ∧
            core.str.Str.trim s = ok s1 ∧
            core.cmp.impls.PartialEqShared.ne
              Str.Insts.CoreCmpPartialEqStr s1 (toStr "0") = ok b) ∨
         (∃ ev, r = core.result.Result.Err ev ∧ b = true))) := by
  intro b
  constructor
  · intro hval
    unfold leveled_enabled at hval
    obtain ⟨r, hgate, hval⟩ := bind_ok_inv _ _ _ hval
    cases r with
    | Ok ov =>
      obtain ⟨s, hd, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨s1, ht, hval⟩ := bind_ok_inv _ _ _ hval
      exact ⟨core.result.Result.Ok ov, hgate, Or.inl ⟨ov, rfl, s, s1, hd, ht, hval⟩⟩
    | Err ev =>
      injection hval with hv
      exact ⟨core.result.Result.Err ev, hgate, Or.inr ⟨ev, rfl, hv.symm⟩⟩
  · rintro ⟨r, hgate, (⟨ov, hok, s, s1, hd, ht, hn⟩ | ⟨ev, herr, hv⟩)⟩
    · subst hok
      unfold leveled_enabled
      exact bind_intro _ hgate (bind_intro s hd (bind_intro s1 ht hn))
    · subst herr
      subst hv
      unfold leveled_enabled
      exact bind_intro _ hgate rfl
