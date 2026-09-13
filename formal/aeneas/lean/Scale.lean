-- Theorems over the Aeneas extract of scale_kernel.rs (RFC-0176).
-- This is the GPS of probes, not the clock.
import Aeneas
import ScaleKernel
open Aeneas.Std Result
open pedra_aeneas_scale_kernel

/-- After settle: probes = levels + L0 covering (catalog entry). -/
theorem point_get_probes_one_plus_one :
    point_get_probes 1#u64 1#u64 = ok (2#u64) := by
  unfold point_get_probes
  have h : core.num.U64.saturating_add 1#u64 1#u64 = 2#u64 := by native_decide
  simp [h]

/-- AS-IS walk returns the file count unchanged. -/
theorem point_get_probes_as_is_is_n_files
    (n_files levels l0 : U64) :
    point_get_probes_as_is n_files levels l0 = ok n_files := by
  unfold point_get_probes_as_is
  rfl

/-- Worst-case as-is is also the file count. -/
theorem probes_worst_as_is_is_n_files
    (n_files levels l0 : U64) :
    probes_worst_as_is n_files levels l0 = ok n_files := by
  unfold probes_worst_as_is
  rfl

/-- Concrete N (RFC-0176 1B-key settle): 4 L1+ levels + L0 trigger 4 = 8
    probes. Unfolds `probes_worst` **and** `point_get_probes`. -/
theorem probes_worst_l0_trigger_via_point_get :
    probes_worst 4#u64 4#u64 = ok (8#u64) ∧
      point_get_probes 4#u64 4#u64 = ok (8#u64) := by
  constructor
  · unfold probes_worst
    unfold point_get_probes
    have h : core.num.U64.saturating_add 4#u64 4#u64 = 8#u64 := by native_decide
    simp [h]
  · unfold point_get_probes
    have h : core.num.U64.saturating_add 4#u64 4#u64 = 8#u64 := by native_decide
    simp [h]

/-- Worst-path clock on that N: `scale_forecast` matches `worst_get_ns`.
    Unfolds `worst_get_ns` **and** `probes_worst`. -/
theorem worst_get_ns_l0_trigger_via_probes_worst :
    worst_get_ns 4#u64 4#u64 = (
      do
        let i ← probes_worst 4#u64 4#u64
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64
          SCALE_WORST_NOISY_BPS
    ) ∧ probes_worst 4#u64 4#u64 = ok (8#u64) := by
  constructor
  · unfold worst_get_ns
    rfl
  · unfold probes_worst
    unfold point_get_probes
    have h : core.num.U64.saturating_add 4#u64 4#u64 = 8#u64 := by native_decide
    simp [h]

/-- AS-IS tooth: walk every live file as a cold disk probe. -/
theorem worst_get_ns_as_is_tooth :
    worst_get_ns_as_is 913#u64 4#u64 4#u64 = (
      predict_get_ns_as_is 913#u64 SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64 0#u64
    ) := by
  unfold worst_get_ns_as_is
  rfl

/-- Happy-path clock: `scale_forecast` matches `happy_get_ns`.
    Unfolds `happy_get_ns` **and** `point_get_probes`. -/
theorem happy_get_ns_l0_best_via_point_get :
    happy_get_ns 4#u64 0#u64 0#u64 = (
      do
        let i ← point_get_probes 4#u64 SCALE_L0_BEST
        let i1 ← happy_hot_bps 0#u64 0#u64
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS i1
          SCALE_HAPPY_NOISY_BPS
    ) ∧ point_get_probes 4#u64 SCALE_L0_BEST = ok (5#u64) := by
  constructor
  · unfold happy_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 4#u64 1#u64 = 5#u64 := by native_decide
    simp [h]

/-- AS-IS tooth: walk every live file as a cold disk probe. -/
theorem happy_get_ns_as_is_tooth :
    happy_get_ns_as_is 913#u64 4#u64 0#u64 0#u64 = (
      predict_get_ns_as_is 913#u64 SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64 0#u64
    ) := by
  unfold happy_get_ns_as_is
  rfl

/-- Best-path clock: `scale_forecast` matches `best_get_ns`.
    Unfolds `best_get_ns` **and** `point_get_probes`. -/
theorem best_get_ns_l0_best_via_point_get :
    best_get_ns 4#u64 = (
      do
        let i ← point_get_probes 4#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) ∧ point_get_probes 4#u64 SCALE_L0_BEST = ok (5#u64) := by
  constructor
  · unfold best_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 4#u64 1#u64 = 5#u64 := by native_decide
    simp [h]

/-- AS-IS tooth: walk every live file as a cold disk probe. -/
theorem best_get_ns_as_is_tooth :
    best_get_ns_as_is 913#u64 4#u64 = (
      predict_get_ns_as_is 913#u64 SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64 0#u64
    ) := by
  unfold best_get_ns_as_is
  rfl

/-- Empty store (keys=0): `scale_forecast` still routes best_ns through
    `best_get_ns`. Unfolds `best_get_ns` **and** `point_get_probes`. -/
theorem scale_forecast_empty_best_via_best_get_ns :
    best_get_ns 0#u64 = (
      do
        let i ← point_get_probes 0#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) ∧ point_get_probes 0#u64 SCALE_L0_BEST = ok (1#u64) := by
  constructor
  · unfold best_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 0#u64 1#u64 = 1#u64 := by native_decide
    simp [h]

/-- `scale_forecast` is the plan rustc links: collect levels, then the three
    clocks. Unfolds `scale_forecast` **and** `best_get_ns`. -/
theorem scale_forecast_is_three_clocks (keys ram_bytes : U64) :
    scale_forecast keys ram_bytes = (
      do
        let store_bytes ← core.num.U64.saturating_mul keys SCALE_BYTES_PER_ENTRY
        let i ← SCALE_L1_BYTES
        let i1 ← level_count store_bytes i
        let levels ← lift (core.convert.num.FromU64U32.from i1)
        let p_best ← point_get_probes levels SCALE_L0_BEST
        let p_worst ← probes_worst levels SCALE_L0_WORST
        let n_files ←
          if i = 0#u64
          then ok 0#u64
          else core.num.U64.div_ceil store_bytes i
        let warm_cap ← warm_cap_bytes ram_bytes
        let happy_hot ← happy_hot_bps store_bytes ram_bytes
        let best_ns ← best_get_ns levels
        let happy_ns ← happy_get_ns levels store_bytes ram_bytes
        let worst_ns ← worst_get_ns levels SCALE_L0_WORST
        ok
          {
            keys,
            ram_bytes,
            store_bytes,
            levels,
            p_best,
            p_worst,
            n_files,
            warm_cap,
            hot := (store_bytes <= warm_cap),
            happy_hot_bps := happy_hot,
            best_ns,
            happy_ns,
            worst_ns
          }
    ) ∧ best_get_ns 0#u64 = (
      do
        let i ← point_get_probes 0#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) := by
  constructor
  · unfold scale_forecast
    rfl
  · unfold best_get_ns
    rfl

/-- RFC-0176 10B-key settle: 5 L1+ levels + L0-best 1 = 6 probes.
    Named test `rfc0176_one_and_ten_billion_stay_log_n`. -/
theorem rfc0176_10b_is_six_probes :
    point_get_probes 5#u64 SCALE_L0_BEST = ok (6#u64) := by
  unfold point_get_probes
  unfold SCALE_L0_BEST
  have h : core.num.U64.saturating_add 5#u64 1#u64 = 6#u64 := by native_decide
  simp [h]

/-- RFC-0176 10B-key worst production: 5 L1+ levels + L0 trigger 4 = 9. -/
theorem rfc0176_10b_worst_is_nine_probes :
    probes_worst 5#u64 4#u64 = ok (9#u64) ∧
      point_get_probes 5#u64 4#u64 = ok (9#u64) := by
  constructor
  · unfold probes_worst
    unfold point_get_probes
    have h : core.num.U64.saturating_add 5#u64 4#u64 = 9#u64 := by native_decide
    simp [h]
  · unfold point_get_probes
    have h : core.num.U64.saturating_add 5#u64 4#u64 = 9#u64 := by native_decide
    simp [h]

/-- RFC-0176 10B-key best clock: `best_get_ns` 5 levels. Unfolds
    `best_get_ns` **and** `point_get_probes`. -/
theorem rfc0176_10b_best_via_best_get_ns :
    best_get_ns 5#u64 = (
      do
        let i ← point_get_probes 5#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) ∧ point_get_probes 5#u64 SCALE_L0_BEST = ok (6#u64) := by
  constructor
  · unfold best_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 5#u64 1#u64 = 6#u64 := by native_decide
    simp [h]

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

/-- RFC-0218 P2.2 (atom `catalog:scale_probes`, entry
    `point_get_probes`): the probes of the point get are EXACTLY the
    saturating_add cited — levels + L0 covering, without walking all the
    files. The AS-IS walks every live SST (lying GPS — tooth
    planted). -/
theorem point_get_probes_fate_iff :
    ∀ (levels l0_covering : U64) (v : U64),
      (point_get_probes levels l0_covering = ok v) ↔
        (v = core.num.U64.saturating_add levels l0_covering) := by
  intro levels l0_covering v
  constructor
  · intro hval
    unfold point_get_probes at hval
    injection hval with hv
    exact hv.symm
  · rintro rfl
    unfold point_get_probes
    rfl

/-- RFC-0218 P2.2 (atom `catalog:scale_probes_worst`, entry
    `probes_worst`): the worst-case production matches is EXACTLY the same adds
    cited — point_get_probes with the trigger full of the L0. The AS-IS still
    walks each file live. -/
theorem probes_worst_fate_iff :
    ∀ (levels l0_max : U64) (v : U64),
      (probes_worst levels l0_max = ok v) ↔
        (point_get_probes levels l0_max = ok v) := by
  intro levels l0_max v
  unfold probes_worst
  exact Iff.rfl

/-- RFC-0218 P2.2 (atom `catalog:scale_happy_hot`, entry
    `happy_hot_bps`): the hot fraction of the happy path is EXACTLY the
    gate cited — the store fitting in the warm_cap returns SCALE_BPS; otherwise the
    residual SCALE_HAPPY_COLD_HOT_BPS. The AS-IS diz 100% always (the
    lie of the 3 TiB — tooth planted). -/
theorem happy_hot_bps_fate_iff :
    ∀ (store_bytes ram_bytes : U64) (v : U64),
      (happy_hot_bps store_bytes ram_bytes = ok v) ↔
        (∃ i : U64, warm_cap_bytes ram_bytes = ok i ∧
          ((store_bytes <= i ∧ v = SCALE_BPS)
           ∨ (¬ (store_bytes <= i) ∧
              v = SCALE_HAPPY_COLD_HOT_BPS))) := by
  intro store_bytes ram_bytes v
  constructor
  · intro hval
    unfold happy_hot_bps at hval
    obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
    refine ⟨i, hi, ?_⟩
    split at hval
    · next hc =>
      injection hval with hv
      exact Or.inl ⟨hc, hv.symm⟩
    · next hc =>
      injection hval with hv
      exact Or.inr ⟨hc, hv.symm⟩
  · rintro ⟨i, hi, (⟨hc, rfl⟩ | ⟨hc, rfl⟩)⟩
    · unfold happy_hot_bps
      refine bind_intro i hi ?_
      rw [if_pos hc]
    · unfold happy_hot_bps
      refine bind_intro i hi ?_
      rw [if_neg hc]

/-- RFC-0218 P2.2 (atom `catalog:scale_warm`, entry
    `warm_cap_bytes`): the ceiling of the WARM is EXACTLY the cited chain —
    ceiling unknown (0) is the floor 3 GiB; otherwise the larger between the
    floor and 3/4 of the ceiling, cut by the reservado (ceiling − 1 GiB).
    The AS-IS ignora the ceiling (u64::MAX — tooth planted). -/
theorem warm_cap_bytes_fate_iff :
    ∀ (ram_ceiling : U64) (v : U64),
      (warm_cap_bytes ram_ceiling = ok v) ↔
        ((ram_ceiling = 0#u64 ∧ WARM_FLOOR_BYTES = ok v)
         ∨ (¬ (ram_ceiling = 0#u64) ∧
            ∃ i share i1 cap i2 reserved : U64,
              core.num.U64.saturating_mul ram_ceiling 3#u64 = ok i ∧
              i / 4#u64 = ok share ∧
              WARM_FLOOR_BYTES = ok i1 ∧
              ((i1 >= share ∧ cap = i1) ∨ (¬ (i1 >= share) ∧ cap = share)) ∧
              WARM_RESERVE_BYTES = ok i2 ∧
              lift (core.num.U64.saturating_sub ram_ceiling i2)
                = ok reserved ∧
              ((cap <= reserved ∧ v = cap)
               ∨ (¬ (cap <= reserved) ∧ v = reserved)))) := by
  intro ram_ceiling v
  constructor
  · intro hval
    unfold warm_cap_bytes at hval
    split at hval
    · next hc => exact Or.inl ⟨hc, hval⟩
    · next hc =>
      obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨share, hshare, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨cap, hcap, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨i2, hi2, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨reserved, hreserved, hval⟩ := bind_ok_inv _ _ _ hval
      refine Or.inr ⟨hc, i, share, i1, cap, i2, reserved, hi, hshare,
        hi1, ?_, hi2, hreserved, ?_⟩
      · split at hcap
        · next hc1 =>
          injection hcap with hcv
          exact Or.inl ⟨hc1, hcv.symm⟩
        · next hc1 =>
          injection hcap with hcv
          exact Or.inr ⟨hc1, hcv.symm⟩
      · split at hval
        · next hc2 =>
          injection hval with hv
          exact Or.inl ⟨hc2, hv.symm⟩
        · next hc2 =>
          injection hval with hv
          exact Or.inr ⟨hc2, hv.symm⟩
  · rintro (⟨rfl, hv⟩ |
      ⟨hc, i, share, i1, cap, i2, reserved, hi, hshare, hi1, hcapd, hi2,
        hreserved, hfin⟩)
    · unfold warm_cap_bytes
      rw [if_pos rfl]
      exact hv
    · unfold warm_cap_bytes
      rw [if_neg hc]
      refine bind_intro i hi
        (bind_intro share hshare (bind_intro i1 hi1 ?_))
      cases hcapd with
      | inl hcapl =>
        obtain ⟨hc1, rfl⟩ := hcapl
        refine bind_intro cap (by rw [if_pos hc1]) ?_
        refine bind_intro i2 hi2 (bind_intro reserved hreserved ?_)
        cases hfin with
        | inl hfinl =>
          obtain ⟨hc2, rfl⟩ := hfinl
          rw [if_pos hc2]
        | inr hfinr =>
          obtain ⟨hc2, rfl⟩ := hfinr
          rw [if_neg hc2]
      | inr hcapr =>
        obtain ⟨hc1, rfl⟩ := hcapr
        refine bind_intro cap (by rw [if_neg hc1]) ?_
        refine bind_intro i2 hi2 (bind_intro reserved hreserved ?_)
        cases hfin with
        | inl hfinl =>
          obtain ⟨hc2, rfl⟩ := hfinl
          rw [if_pos hc2]
        | inr hfinr =>
          obtain ⟨hc2, rfl⟩ := hfinr
          rw [if_neg hc2]

/-- RFC-0218 P2.2 (atom `catalog:scale_predict`, entry
    `predict_get_ns`): the clock predicts is EXACTLY the cited chain
    — clamps min(hot, SCALE_BPS)/min(noisy, 9000), a mix
    hot·τ_ram + (SCALE_BPS−hot)·τ_disk, a conta em u128, a taxa
    (1+η) and the try_from with ceiling u64::MAX. The AS-IS ignora η (tooth
    planted). -/
theorem predict_get_ns_fate_iff :
    ∀ (probes tau_ram_ns tau_disk_ns hot_bps noisy_bps : U64) (v : U64),
      (predict_get_ns probes tau_ram_ns tau_disk_ns hot_bps noisy_bps
          = ok v) ↔
        (∃ hot noisy i i1 i2 mix : U64,
           core.cmp.Ord.min.trait_default core.cmp.OrdU64 hot_bps SCALE_BPS
             = ok hot ∧
           core.cmp.Ord.min.trait_default core.cmp.OrdU64 noisy_bps
             9000#u64 = ok noisy ∧
           core.num.U64.saturating_mul hot tau_ram_ns = ok i ∧
           SCALE_BPS - hot = ok i1 ∧
           core.num.U64.saturating_mul i1 tau_disk_ns = ok i2 ∧
           lift (core.num.U64.saturating_add i i2) = ok mix ∧
         ∃ i3 i4 i5 i6 per i7 i8 i9 i10 i11 taxed : U128,
           lift (core.convert.num.FromU128U64.from probes) = ok i3 ∧
           lift (core.convert.num.FromU128U64.from mix) = ok i4 ∧
           core.num.U128.saturating_mul i3 i4 = ok i5 ∧
           lift (core.convert.num.FromU128U64.from SCALE_BPS) = ok i6 ∧
           i5 / i6 = ok per ∧
           lift (core.convert.num.FromU128U64.from SCALE_BPS) = ok i7 ∧
           lift (core.convert.num.FromU128U64.from noisy) = ok i8 ∧
           i7 + i8 = ok i9 ∧
           core.num.U128.saturating_mul per i9 = ok i10 ∧
           lift (core.convert.num.FromU128U64.from SCALE_BPS) = ok i11 ∧
           i10 / i11 = ok taxed ∧
         ∃ r : core.result.Result U64 core.num.error.TryFromIntError,
           U64.Insts.CoreConvertTryFromU128TryFromIntError.try_from taxed
             = ok r ∧
         ∃ w : U64,
           core.result.Result.unwrap_or r core.num.U64.MAX = ok w ∧
           v = w) := by
  intro probes tau_ram_ns tau_disk_ns hot_bps noisy_bps v
  constructor
  · intro hval
    unfold predict_get_ns at hval
    obtain ⟨hot, hhot, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨noisy, hnoisy, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i2, hi2, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨mix, hmix, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i3, hi3, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i4, hi4, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i5, hi5, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i6, hi6, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨per, hper, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i7, hi7, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i8, hi8, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i9, hi9, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i10, hi10, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i11, hi11, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨taxed, htaxed, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨r, hr, hval⟩ := bind_ok_inv _ _ _ hval
    exact ⟨hot, noisy, i, i1, i2, mix, hhot, hnoisy, hi, hi1, hi2, hmix,
      i3, i4, i5, i6, per, i7, i8, i9, i10, i11, taxed, hi3, hi4, hi5,
      hi6, hper, hi7, hi8, hi9, hi10, hi11, htaxed, r, hr, v, hval, rfl⟩
  · rintro ⟨hot, noisy, i, i1, i2, mix, hhot, hnoisy, hi, hi1, hi2, hmix,
      i3, i4, i5, i6, per, i7, i8, i9, i10, i11, taxed, hi3, hi4, hi5,
      hi6, hper, hi7, hi8, hi9, hi10, hi11, htaxed, r, hr, w, hw, hv⟩
    unfold predict_get_ns
    refine bind_intro hot hhot (bind_intro noisy hnoisy
      (bind_intro i hi (bind_intro i1 hi1 (bind_intro i2 hi2
      (bind_intro mix hmix (bind_intro i3 hi3 (bind_intro i4 hi4
      (bind_intro i5 hi5 (bind_intro i6 hi6 (bind_intro per hper
      (bind_intro i7 hi7 (bind_intro i8 hi8 (bind_intro i9 hi9
      (bind_intro i10 hi10 (bind_intro i11 hi11 (bind_intro taxed htaxed
      (bind_intro r hr ?_)))))))))))))))))
    rw [hv]
    exact hw

/-- RFC-0218 P2.2 (atom `catalog:scale_forecast`, entry
    `scale_forecast`): the table RFC-0176 is EXACTLY the composition
    cited — each field is the atom of the kernel corresponding
    (saturating_mul, level_count the callee cited, point_get_probes,
    probes_worst, div_ceil, warm_cap_bytes, happy_hot_bps, best/happy/
    worst_get_ns); hot is the gate store <= warm_cap. The AS-IS walks all
    the files and always says hot (tooth planted). -/
theorem scale_forecast_fate_iff :
    ∀ (keys ram_bytes : U64) (r : ScaleForecast),
      (scale_forecast keys ram_bytes = ok r) ↔
        (∃ store_bytes i : U64,
           core.num.U64.saturating_mul keys SCALE_BYTES_PER_ENTRY
             = ok store_bytes ∧
           SCALE_L1_BYTES = ok i ∧
         ∃ i1 : U32,
           level_count store_bytes i = ok i1 ∧
         ∃ levels p_best p_worst n_files warm_cap happy_hot best_ns
           happy_ns worst_ns : U64,
           lift (core.convert.num.FromU64U32.from i1) = ok levels ∧
           point_get_probes levels SCALE_L0_BEST = ok p_best ∧
           probes_worst levels SCALE_L0_WORST = ok p_worst ∧
           ((i = 0#u64 ∧ n_files = 0#u64)
            ∨ (¬ (i = 0#u64) ∧
               core.num.U64.div_ceil store_bytes i = ok n_files)) ∧
           warm_cap_bytes ram_bytes = ok warm_cap ∧
           happy_hot_bps store_bytes ram_bytes = ok happy_hot ∧
           best_get_ns levels = ok best_ns ∧
           happy_get_ns levels store_bytes ram_bytes = ok happy_ns ∧
           worst_get_ns levels SCALE_L0_WORST = ok worst_ns ∧
           r = ScaleForecast.mk keys ram_bytes store_bytes levels p_best
             p_worst n_files warm_cap (decide (store_bytes <= warm_cap))
             happy_hot best_ns happy_ns worst_ns) := by
  intro keys ram_bytes r
  constructor
  · intro hval
    unfold scale_forecast at hval
    obtain ⟨store_bytes, hsb, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨levels, hlevels, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨p_best, hpb, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨p_worst, hpw, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨n_files, hnf, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨warm_cap, hwc, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨happy_hot, hhh, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨best_ns, hbn, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨happy_ns, hhns, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨worst_ns, hwns, hval⟩ := bind_ok_inv _ _ _ hval
    injection hval with hr
    refine ⟨store_bytes, i, hsb, hi, i1, hi1, levels, p_best, p_worst,
      n_files, warm_cap, happy_hot, best_ns, happy_ns, worst_ns, hlevels,
      hpb, hpw, ?_, hwc, hhh, hbn, hhns, hwns, hr.symm⟩
    split at hnf
    · next hc =>
      injection hnf with hv
      exact Or.inl ⟨hc, hv.symm⟩
    · next hc => exact Or.inr ⟨hc, hnf⟩
  · rintro ⟨store_bytes, i, hsb, hi, i1, hi1, levels, p_best, p_worst,
      n_files, warm_cap, happy_hot, best_ns, happy_ns, worst_ns, hlevels,
      hpb, hpw, hdisj, hwc, hhh, hbn, hhns, hwns, hr⟩
    unfold scale_forecast
    refine bind_intro store_bytes hsb (bind_intro i hi (bind_intro i1 hi1
      (bind_intro levels hlevels (bind_intro p_best hpb
      (bind_intro p_worst hpw ?_)))))
    · cases hdisj with
      | inl hd =>
        obtain ⟨hc, rfl⟩ := hd
        refine bind_intro 0#u64 (by rw [if_pos hc]) (bind_intro warm_cap
          hwc (bind_intro happy_hot hhh (bind_intro best_ns hbn
          (bind_intro happy_ns hhns (bind_intro worst_ns hwns ?_)))))
        rw [hr]
      | inr hd =>
        obtain ⟨hc, hnf⟩ := hd
        refine bind_intro n_files (by rw [if_neg hc]; exact hnf)
          (bind_intro warm_cap hwc (bind_intro happy_hot hhh
          (bind_intro best_ns hbn (bind_intro happy_ns hhns
          (bind_intro worst_ns hwns ?_)))))
        rw [hr]
