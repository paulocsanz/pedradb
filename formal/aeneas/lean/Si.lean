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
