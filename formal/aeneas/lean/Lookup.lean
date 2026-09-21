-- Theorems over Aeneas extract of lookup_kernel.rs
import Aeneas
import LookupKernel
open Aeneas.Std Result
open pedra_aeneas_lookup_kernel

theorem snap_is_empty_zero :
    snap_is_empty 0#u64 = ok true := by
  unfold snap_is_empty
  rfl

theorem snap_is_empty_as_is_tooth :
    snap_is_empty_as_is 0#u64 = ok false := by
  unfold snap_is_empty_as_is
  rfl

/-- RFC-0213 P0.2 (storage cadence, atom `catalog:snap_empty`):
    a snapshot is empty EXACTLY when its sequence is zero — fate
    forall over the extracted body (RFC-0170 P2.4); the AS-IS mutant
    never sees an empty snapshot (the lie the DST plant
    `snap_is_empty_on_live_zero_is_not_ok` refutes). -/
theorem snap_empty_fate_iff :
    ∀ (seq : U64) (v : Bool),
      (snap_is_empty seq = ok v) ↔ v = decide (seq = 0#u64) := by
  intro seq v
  unfold snap_is_empty
  simp
  exact eq_comm

/-- RFC-0213 P0.2 (storage cadence, atom `catalog:snap_below_watermark`):
    a snapshot is below the watermark EXACTLY when its sequence is
    older than the earliest visible — fate forall over the extracted
    body (RFC-0170 P2.4); the AS-IS mutant never drops below (the
    lie the DST plant `snap_below_watermark_on_live_below_is_not_ok`
    refutes). -/
theorem snap_below_watermark_fate_iff :
    ∀ (seq earliest : U64) (v : Bool),
      (snap_below_watermark seq earliest = ok v) ↔
        v = decide (seq < earliest) := by
  intro seq earliest v
  unfold snap_below_watermark
  simp
  exact eq_comm

/-- RFC-0213 P0.2 (storage cadence, atom `catalog:mem_point_decides`):
    the memtable point verdict is the hit flag itself — fate forall
    over the extracted body (RFC-0170 P2.4); the AS-IS mutant always
    reports a miss (the lie the DST plant
    `mem_point_decides_on_live_hit_is_not_ok` refutes). -/
theorem mem_point_decides_fate_iff :
    ∀ (has_point v : Bool),
      (mem_point_decides has_point = ok v) ↔ v = has_point := by
  intro has_point v
  unfold mem_point_decides
  cases has_point <;> cases v <;> simp

/-- RFC-0213 P0.2 (storage cadence, atom `catalog:prefer_newer_seq`):
    a candidate wins EXACTLY when there is no incumbent best, or the
    incumbent exists and the candidate sequence is newer — fate
    forall over the extracted body (RFC-0170 P2.4); the AS-IS mutant
    prefers everything, older included (the lie the DST plant
    `prefer_newer_seq_on_live_older_first_is_not_ok` refutes). -/
theorem prefer_newer_seq_fate_iff :
    ∀ (have_best : Bool) (new_seq best_seq : U64) (v : Bool),
      (prefer_newer_seq have_best new_seq best_seq = ok v) ↔
        ((v = decide (new_seq > best_seq) ∧ have_best = true)
          ∨ (v = true ∧ have_best = false)) := by
  intro have_best new_seq best_seq v
  unfold prefer_newer_seq
  cases have_best <;> simp <;> exact eq_comm

/-- RFC-0219 P1.1 (atom `catalog:point_cache_validity`): o fill/hit do
    point/prefix cache é admissível EXATAMENTE enquanto o published seq
    ainda é igual ao seq em que a resposta foi computada — publish
    avançou ⇒ resposta pré-publish é velha e não entra (F198/F207). O
    AS-IS cacheia sempre (resposta velha congelada — tooth plantado). -/
theorem point_cache_validity_fate_iff :
    ∀ (published answer : U64) (plan : PointCachePlan),
      (point_cache_validity published answer = ok plan) ↔
        ((published = answer ∧ plan = PointCachePlan.CacheCurrent) ∨
          (published ≠ answer ∧ plan = PointCachePlan.PublishAdvanced)) := by
  intro published answer plan
  simp only [point_cache_validity]
  split <;> rename_i c
  · constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨c, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2)
      · subst hv
        rfl
      · exact absurd c h2.1
  · constructor
    · intro hval
      injection hval with hv
      exact Or.inr ⟨c, hv.symm⟩
    · rintro (h1 | ⟨-, hv⟩)
      · exact absurd h1.1 c
      · subst hv
        rfl

/-- RFC-0219 P1.1 (atom `catalog:point_tombstone`): um ponto achado é
    servido EXATAMENTE quando nenhum range tombstone cobre — tombstone
    cobrindo (t.seq > point_seq) sombreia o valor e o caller lê Deleted
    (RFC-0150). O AS-IS nunca sombreia (ressurreição — tooth plantado). -/
theorem point_tombstone_plan_fate_iff :
    ∀ (range_hidden : Bool) (plan : PointTombstonePlan),
      (point_tombstone_plan range_hidden = ok plan) ↔
        ((range_hidden = true ∧ plan = PointTombstonePlan.ShadowedDeleted) ∨
          (range_hidden = false ∧ plan = PointTombstonePlan.ValueVisible)) := by
  intro range_hidden plan
  simp only [point_tombstone_plan]
  split <;> rename_i c
  · constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨c, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2)
      · subst hv
        rfl
      · exact absurd h2.1 (by simp [*])
  · rw [Bool.not_eq_true] at c
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr ⟨c, hv.symm⟩
    · rintro (h1 | ⟨-, hv⟩)
      · exact absurd h1.1 (by simp [*])
      · subst hv
        rfl
