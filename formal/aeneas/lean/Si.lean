-- Theorems over Aeneas extract of si_kernel.rs
-- (RFC-0002 P19/P21/P30/F42/F55/F84). Payment is the linked rustc bodies;
-- the former cfg(verus_keep_ghost) stand-in was deleted. No holes here.
import Aeneas
import SiKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_si_kernel

/-- F42 teeth: a live reader beats a dead one. -/
theorem si_reader_beats_c_live :
    si_reader_beats true true false 0#u64 false false false 0#u64 = ok true := by
  unfold si_reader_beats
  rfl

/-- F42 teeth: participation breaks the next tie. -/
theorem si_reader_beats_participation_breaks_tie :
    si_reader_beats true true false 0#u64 true false false 0#u64 = ok true := by
  unfold si_reader_beats
  rfl

/-- F42 teeth: self breaks the next tie. -/
theorem si_reader_beats_self_breaks_next_tie :
    si_reader_beats true true true 0#u64 true true false 0#u64 = ok true := by
  unfold si_reader_beats
  rfl

/-- F42 teeth: the applied watermark breaks the last tie. -/
theorem si_reader_beats_watermark_breaks_last_tie :
    si_reader_beats true true true 5#u64 true true true 3#u64 = ok true := by
  unfold si_reader_beats
  have h : (5#u64 > 3#u64) = true := by native_decide
  simp [h]

/-- AS-IS F42 dente: the fold never advances the reader. -/
theorem si_reader_beats_as_is_dente :
    si_reader_beats_as_is true true false 0#u64 false false false 0#u64
      = ok false := by
  rfl

/-- F84 teeth: point get prefers the applied watermark. -/
theorem point_get_prefer_applied_teeth :
    point_get_prefer_applied = ok true := by
  rfl

/-- AS-IS F84 dente: point get rides the global sequence. -/
theorem point_get_prefer_applied_as_is_dente :
    point_get_prefer_applied_as_is = ok false := by
  rfl

/-- F84 teeth: the point-get watermark is the range applied. -/
theorem point_get_watermark_prefers_range_applied :
    point_get_watermark (7#u64) (9#u64) = ok 7#u64 := by
  rfl

/-- AS-IS F84 dente: the watermark is the global sequence instead. -/
theorem point_get_watermark_as_is_dente :
    point_get_watermark_as_is (7#u64) (9#u64) = ok 9#u64 := by
  rfl

/-- RFC-0218 P1.3 1/11 (átomo `catalog:point_get_prefer`, entrada
    `point_get_prefer_applied`): o point-get prefere o índice applied
    — EXATAMENTE a constante citada true. O AS-IS é false (applied
    ignorado — dente plantado). -/
theorem point_get_prefer_applied_fate_iff :
    ∀ (v : Bool),
      (point_get_prefer_applied = ok v) ↔ (v = true) := by
  intro v
  constructor
  · intro hval
    unfold point_get_prefer_applied at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P1.3 5/11 (átomo `catalog:point_get_wm`, entrada
    `point_get_watermark`): o watermark do point-get é EXATAMENTE o
    lift citado `range_applied` — o global_seq não entra. O AS-IS
    devolve global_seq (ler não-aplicado — dente plantado). -/
theorem point_get_watermark_fate_iff :
    ∀ (range_applied : U64) (global_seq : U64) (r : U64),
      (point_get_watermark range_applied global_seq = ok r) ↔
      (r = range_applied) := by
  intro range_applied global_seq r
  constructor
  · intro hval
    unfold point_get_watermark at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
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

/-- RFC-0218 P1.3 8/11 (átomo `catalog:si_read`, entrada
    `snapshot_read_plan`): servir ou recusar um snapshot é EXATAMENTE
    compará-lo contra o piso citado `watermark - 1` (saturating) —
    abaixo do piso, TooOld (fail closed); no piso ou acima, Serve. O
    AS-IS serve todo mundo (ausência fabricada — dente plantado). -/
theorem snapshot_read_plan_fate_iff :
    ∀ (snapshot : U64) (watermark : U64) (r : SnapshotRead),
      (snapshot_read_plan snapshot watermark = ok r) ↔
        (∃ i : U64, lift (core.num.U64.saturating_sub watermark 1#u64) = ok i ∧
          ((i > snapshot ∧ r = SnapshotRead.TooOld) ∨
           (¬ (i > snapshot) ∧ r = SnapshotRead.Serve))) := by
  intro snapshot watermark r
  constructor
  · intro hval
    unfold snapshot_read_plan at hval
    obtain ⟨i, hgate, hval⟩ := bind_ok_inv _ _ _ hval
    refine ⟨i, hgate, ?_⟩
    split at hval
    · next hc =>
      injection hval with hv
      exact Or.inl ⟨hc, hv.symm⟩
    · next hc =>
      injection hval with hv
      exact Or.inr ⟨hc, hv.symm⟩
  · rintro ⟨i, hgate, (⟨hc, hv⟩ | ⟨hc, hv⟩)⟩
    · unfold snapshot_read_plan
      refine bind_intro i hgate ?_
      subst hv
      rw [if_pos hc]
    · unfold snapshot_read_plan
      refine bind_intro i hgate ?_
      subst hv
      rw [if_neg hc]

/-- RFC-0218 P1.3 9/11 (átomo `catalog:si_reader`, entrada
    `si_reader_beats`): eleger o leitor SI é EXATAMENTE a cascata
    citada — liveness (líder+participante) decide primeiro; empatada,
    participação; empatada, self; empatada, o watermark applied
    decide. O AS-IS nunca avança o leitor (dente plantado). -/
theorem si_reader_beats_fate_iff :
    ∀ (c_leader c_part c_self : Bool) (c_applied : U64)
      (b_leader b_part b_self : Bool) (b_applied : U64) (v : Bool),
      (si_reader_beats c_leader c_part c_self c_applied
          b_leader b_part b_self b_applied = ok v) ↔
        (∃ c_live b_live : Bool,
          ((if c_leader = true then ok c_part else ok false) = ok c_live ∧
           (if b_leader = true then ok b_part else ok false) = ok b_live ∧
          (((c_live != b_live) = true ∧ v = c_live) ∨
           ((c_live != b_live) = false ∧
            (((c_part != b_part) = true ∧ v = c_part) ∨
             ((c_part != b_part) = false ∧
              (((c_self != b_self) = true ∧ v = c_self) ∨
               ((c_self != b_self) = false ∧
                v = decide (c_applied > b_applied))))))))) := by
  intro c_leader c_part c_self c_applied b_leader b_part b_self b_applied v
  constructor
  · intro hval
    unfold si_reader_beats at hval
    obtain ⟨c_live, hcl, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨b_live, hbl, hval⟩ := bind_ok_inv _ _ _ hval
    refine ⟨c_live, b_live, hcl, hbl, ?_⟩
    split at hval
    · next hc =>
      injection hval with hv
      exact Or.inl ⟨hc, hv.symm⟩
    · next hc =>
      simp only [Bool.not_eq_true] at hc
      split at hval
      · next hc2 =>
        injection hval with hv
        exact Or.inr ⟨hc, Or.inl ⟨hc2, hv.symm⟩⟩
      · next hc2 =>
        simp only [Bool.not_eq_true] at hc2
        split at hval
        · next hc3 =>
          injection hval with hv
          exact Or.inr ⟨hc, Or.inr ⟨hc2, Or.inl ⟨hc3, hv.symm⟩⟩⟩
        · next hc3 =>
          simp only [Bool.not_eq_true] at hc3
          injection hval with hv
          exact Or.inr ⟨hc, Or.inr ⟨hc2, Or.inr ⟨hc3, hv.symm⟩⟩⟩
  · rintro ⟨c_live, b_live, hcl, hbl, htree⟩
    unfold si_reader_beats
    refine bind_intro c_live hcl (bind_intro b_live hbl ?_)
    rcases htree with ⟨hc, hv⟩ | ⟨hc, h2⟩
    · subst hv
      rw [if_pos hc]
    · rcases h2 with ⟨hc2, hv⟩ | ⟨hc2, h3⟩
      · subst hv
        have h1 : ¬((c_live != b_live) = true) := by simp [hc]
        rw [if_neg h1, if_pos hc2]
      · rcases h3 with ⟨hc3, hv⟩ | ⟨hc3, hv⟩
        · subst hv
          have h1 : ¬((c_live != b_live) = true) := by simp [hc]
          have h2 : ¬((c_part != b_part) = true) := by simp [hc2]
          rw [if_neg h1, if_neg h2, if_pos hc3]
        · subst hv
          have h1 : ¬((c_live != b_live) = true) := by simp [hc]
          have h2 : ¬((c_part != b_part) = true) := by simp [hc2]
          have h3 : ¬((c_self != b_self) = true) := by simp [hc3]
          rw [if_neg h1, if_neg h2, if_neg h3]
