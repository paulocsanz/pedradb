-- Cross-lib: the storage write path (RFC-0213 P2.2) — admission →
-- WAL plan → torn-tail cut COMPOSED over the three registered atoms
-- (`catalog:write_admit`, `catalog:wal_commit_plan`,
-- `catalog:torn_tail_needs_cut`). Each leg is derived from its
-- registered iff atom; the extracted bodies are never opened.
-- Registration rule: a row needs a single catalog pair/entry; this
-- composition spans three kernels — same reason the other compose
-- libs carry no row (reason dated in findings).
import Aeneas
import WriteAdmission
open Aeneas.Std Result
open pedra_aeneas_write_admission_kernel

/-- An ok-valued Bool chain lands exactly along its fate pair:
    `ok b = ok v` iff (v true along P) or (v false along Q), with Q
    following from ¬P and Q refuting P. -/
private theorem ok_iff_fate {b v : Bool} (P Q : Prop)
    (ht : P → b = true) (hf : ¬P → b = false) (hq : ¬P → Q)
    (hqp : Q → ¬P) :
    ((ok b : Result Bool) = ok v) ↔
      ((v = true ∧ P) ∨ (v = false ∧ Q)) := by
  by_cases hp : P
  · rw [ht hp]
    constructor
    · intro h
      injection h with hv
      exact Or.inl ⟨hv.symm, hp⟩
    · rintro (⟨rfl, _⟩ | ⟨rfl, hnp⟩)
      · rfl
      · exact absurd hp (hqp hnp)
  · rw [hf hp]
    constructor
    · intro h
      injection h with hv
      exact Or.inr ⟨hv.symm, hq hp⟩
    · rintro (⟨rfl, hnp⟩ | ⟨rfl, _⟩)
      · exact absurd hnp hp
      · rfl

/-- RFC-0213 P2.2: the published tail — with admission Ok and the
    plan off the fence, the record survives recovery EXACTLY when it
    is not in the torn tail — derived from the registered
    `torn_tail_needs_cut` atom (shared by both publish plans). -/
private theorem published_tail_fate (len last_good : U64) (v : Bool)
    (A B C : Prop) (hA : ¬A) (hB : ¬B) (hC : ¬C) :
    (Aeneas.Std.bind (torn_tail_needs_cut len last_good)
        (fun cut => ok (!cut)) = ok v) ↔
      ((v = true ∧ ¬A ∧ ¬B ∧ ¬C ∧ ¬(len > last_good))
        ∨ (v = false ∧ (A ∨ B ∨ C ∨ (len > last_good)))) := by
  by_cases hD : len > last_good
  · have h3 : torn_tail_needs_cut len last_good = ok true :=
      (torn_tail_needs_cut_fate_iff len last_good true).mpr (by simp [hD])
    rw [h3]
    show ((ok false : Result Bool) = ok v) ↔ _
    exact ok_iff_fate (¬A ∧ ¬B ∧ ¬C ∧ ¬(len > last_good)) (A ∨ B ∨ C ∨ (len > last_good))
      (fun hp => absurd hD hp.2.2.2) (fun _ => rfl)
      (fun _ => Or.inr (Or.inr (Or.inr hD)))
      (fun _ => fun hp => absurd hD hp.2.2.2)
  · have h3 : torn_tail_needs_cut len last_good = ok false :=
      (torn_tail_needs_cut_fate_iff len last_good false).mpr (by simp [hD])
    rw [h3]
    show ((ok true : Result Bool) = ok v) ↔ _
    exact ok_iff_fate (¬A ∧ ¬B ∧ ¬C ∧ ¬(len > last_good)) (A ∨ B ∨ C ∨ (len > last_good))
      (fun _ => rfl)
      (fun hnp => False.elim (hnp ⟨hA, hB, hC, hD⟩))
      (fun hnp => False.elim (hnp ⟨hA, hB, hC, hD⟩))
      (fun q _ => q.elim hA (fun b => b.elim hB (fun c => c.elim hC hD)))

/-- RFC-0213 P2.2: the storage write path COMPOSED — admission gates
    the append (no Ok ⇒ the write never reaches the WAL), the WAL plan
    fences a failed required sync (Fence ⇒ no publish), and recovery
    cuts the torn tail (the record survives EXACTLY when it is not
    past last-good). The composed chain lands `ok v` with v true
    EXACTLY along the four-way conjunction, for EVERY input — each leg
    from its registered atom, zero holes. -/
theorem storage_write_path_recovered_iff :
    ∀ (mem_bytes : U64) (mem_armed : Bool) (mem_limit l0 : U64)
      (l0_armed : Bool) (l0_limit : U64)
      (need_sync sync_failed : Bool) (len last_good : U64) (v : Bool),
      (Aeneas.Std.bind
        (write_admit mem_bytes mem_armed mem_limit l0 l0_armed l0_limit)
        (fun adm =>
          match adm with
          | WriteAdmit.Ok =>
              Aeneas.Std.bind (wal_commit_plan need_sync sync_failed)
                (fun plan =>
                  match plan with
                  | WalCommitPlan.AppendSyncFence => ok false
                  | _ =>
                      Aeneas.Std.bind (torn_tail_needs_cut len last_good)
                        (fun cut => ok (!cut)))
          | WriteAdmit.StallMem => ok false
          | WriteAdmit.StallL0 => ok false) = ok v) ↔
        ((v = true ∧
            ¬(mem_armed = true ∧ mem_bytes >= mem_limit) ∧
            ¬(l0_armed = true ∧ l0 >= l0_limit) ∧
            ¬(need_sync = true ∧ sync_failed = true) ∧
            ¬(len > last_good))
          ∨ (v = false ∧
            ((mem_armed = true ∧ mem_bytes >= mem_limit)
              ∨ (l0_armed = true ∧ l0 >= l0_limit)
              ∨ (need_sync = true ∧ sync_failed = true)
              ∨ (len > last_good)))) := by
  intro mem_bytes mem_armed mem_limit l0 l0_armed l0_limit
       need_sync sync_failed len last_good v
  have hadm := write_admit_fate_iff mem_bytes mem_armed mem_limit
    l0 l0_armed l0_limit
  have hplan := wal_commit_plan_fate_iff need_sync sync_failed
  by_cases hA : mem_armed = true ∧ mem_bytes >= mem_limit
  · have h1 : write_admit mem_bytes mem_armed mem_limit l0 l0_armed l0_limit
        = ok WriteAdmit.StallMem :=
      (hadm WriteAdmit.StallMem).mpr (Or.inl ⟨rfl, hA.1, hA.2⟩)
    rw [h1]
    show ((ok false : Result Bool) = ok v) ↔ _
    exact ok_iff_fate (¬(mem_armed = true ∧ mem_bytes >= mem_limit)
        ∧ ¬(l0_armed = true ∧ l0 >= l0_limit)
        ∧ ¬(need_sync = true ∧ sync_failed = true) ∧ ¬(len > last_good))
      ((mem_armed = true ∧ mem_bytes >= mem_limit)
        ∨ (l0_armed = true ∧ l0 >= l0_limit)
        ∨ (need_sync = true ∧ sync_failed = true) ∨ (len > last_good))
      (fun hp => absurd hA hp.1) (fun _ => rfl) (fun _ => Or.inl hA)
      (fun _ => fun hp => absurd hA hp.1)
  · by_cases hB : l0_armed = true ∧ l0 >= l0_limit
    · have h1 : write_admit mem_bytes mem_armed mem_limit l0 l0_armed l0_limit
          = ok WriteAdmit.StallL0 :=
        (hadm WriteAdmit.StallL0).mpr (Or.inr (Or.inl ⟨rfl, hB.1, hB.2, hA⟩))
      rw [h1]
      show ((ok false : Result Bool) = ok v) ↔ _
      exact ok_iff_fate (¬(mem_armed = true ∧ mem_bytes >= mem_limit)
          ∧ ¬(l0_armed = true ∧ l0 >= l0_limit)
          ∧ ¬(need_sync = true ∧ sync_failed = true) ∧ ¬(len > last_good))
        ((mem_armed = true ∧ mem_bytes >= mem_limit)
          ∨ (l0_armed = true ∧ l0 >= l0_limit)
          ∨ (need_sync = true ∧ sync_failed = true) ∨ (len > last_good))
        (fun hp => absurd hB hp.2.1) (fun _ => rfl)
        (fun _ => Or.inr (Or.inl hB))
        (fun _ => fun hp => absurd hB hp.2.1)
    · have h1 : write_admit mem_bytes mem_armed mem_limit l0 l0_armed l0_limit
          = ok WriteAdmit.Ok :=
        (hadm WriteAdmit.Ok).mpr (Or.inr (Or.inr ⟨rfl, hA, hB⟩))
      by_cases hC : need_sync = true ∧ sync_failed = true
      · have h2 : wal_commit_plan need_sync sync_failed
            = ok WalCommitPlan.AppendSyncFence :=
          (hplan WalCommitPlan.AppendSyncFence).mpr (Or.inl ⟨rfl, hC.1, hC.2⟩)
        rw [h1, h2]
        show ((ok false : Result Bool) = ok v) ↔ _
        exact ok_iff_fate (¬(mem_armed = true ∧ mem_bytes >= mem_limit)
            ∧ ¬(l0_armed = true ∧ l0 >= l0_limit)
            ∧ ¬(need_sync = true ∧ sync_failed = true) ∧ ¬(len > last_good))
          ((mem_armed = true ∧ mem_bytes >= mem_limit)
            ∨ (l0_armed = true ∧ l0 >= l0_limit)
            ∨ (need_sync = true ∧ sync_failed = true) ∨ (len > last_good))
          (fun hp => absurd hC hp.2.2.1) (fun _ => rfl)
          (fun _ => Or.inr (Or.inr (Or.inl hC)))
          (fun _ => fun hp => absurd hC hp.2.2.1)
      · by_cases hns : need_sync = true
        · have hsf : sync_failed = false := by
            cases hsf : sync_failed with
            | false => rfl
            | true => exact absurd ⟨hns, hsf⟩ hC
          have h2 : wal_commit_plan need_sync sync_failed
              = ok WalCommitPlan.AppendSyncApplyOk :=
            (hplan WalCommitPlan.AppendSyncApplyOk).mpr
              (Or.inr (Or.inl ⟨rfl, hns, hsf⟩))
          rw [h1, h2]
          exact published_tail_fate len last_good v _ _ _ hA hB hC
        · have hnsf : need_sync = false := by
            cases hnsb : need_sync with
            | false => rfl
            | true => exact absurd hnsb hns
          have h2 : wal_commit_plan need_sync sync_failed
              = ok WalCommitPlan.AppendApplyOk :=
            (hplan WalCommitPlan.AppendApplyOk).mpr
              (Or.inr (Or.inr ⟨rfl, hnsf⟩))
          rw [h1, h2]
          exact published_tail_fate len last_good v _ _ _ hA hB hC
