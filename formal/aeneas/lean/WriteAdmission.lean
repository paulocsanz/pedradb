-- Theorems over the Aeneas extract of production write_admission_kernel.rs
-- (RFC-0170 P2.1). Fail-closed: this file must not contain a hole.
import Aeneas
import WriteAdmissionKernel
open Aeneas.Std Result
open pedra_aeneas_write_admission_kernel

/-- Extracted idle gate is true iff every stall knob is off. -/
theorem write_admission_idle_matches_spec :
    write_admission_idle false false false = ok true := by
  unfold write_admission_idle
  rfl

/-- A live mem-stall knob refuses the idle path. -/
theorem write_admission_idle_mem_stall_refuses :
    write_admission_idle true false false = ok false := by
  unfold write_admission_idle
  rfl

/-- AS-IS dente: stall knobs are ignored (always idle). -/
theorem write_admission_idle_as_is_dente :
    write_admission_idle_as_is true true true = ok true := by
  unfold write_admission_idle_as_is
  rfl

/-- Hard admit: mem over an armed limit is StallMem. -/
theorem write_admit_mem_over_stalls :
    write_admit 100#u64 true 50#u64 0#u64 false 0#u64
      = ok WriteAdmit.StallMem := by
  unfold write_admit
  have h : (100#u64 ≥ 50#u64) = true := by native_decide
  simp [h]

/-- AS-IS dente: mem over still admits. -/
theorem write_admit_as_is_dente :
    write_admit_as_is 100#u64 true 50#u64 8#u64 true 4#u64
      = ok WriteAdmit.Ok := by
  unfold write_admit_as_is
  rfl

/-- Put-Ok: client WriteOptions.sync=true requires a WAL barrier. -/
theorem wal_sync_required_client_true :
    wal_sync_required true true false = ok true := by
  unfold wal_sync_required
  rfl

/-- Sync-knob resolution is total and single-valued: the extracted gate
    returns ok on exactly one value — the client's explicit choice when
    it set one, else the db-level default (registered atom, RFC-0200 P2.1). -/
theorem wal_sync_required_ok_iff_client_else_db :
    ∀ (client_set client_sync db_sync v : Bool),
      (wal_sync_required client_set client_sync db_sync = ok v) ↔
      (if client_set then v = client_sync else v = db_sync) := by
  intro client_set client_sync db_sync v
  unfold wal_sync_required
  cases client_set with
  | true => simp [eq_comm]
  | false => simp [eq_comm]

/-- Open-options sync requires a directory fsync after rename/create. -/
theorem dir_sync_required_when_sync :
    dir_sync_required true = ok true := by
  unfold dir_sync_required
  rfl

/-- Put-Ok script: required sync that succeeded is Sync before Apply/Ok. -/
theorem wal_commit_plan_need_sync_ok :
    wal_commit_plan true false = ok WalCommitPlan.AppendSyncApplyOk := by
  unfold wal_commit_plan
  rfl

/-- Required sync failed ⇒ Fence (no Apply/Ok). Unfolds the plan rustc
    links **and** `fence_on_sync_fail` (the callee the plan now calls). -/
theorem wal_commit_plan_fence_via_fence_on_sync_fail :
    fence_on_sync_fail true true = ok true ∧
      wal_commit_plan true true = ok WalCommitPlan.AppendSyncFence := by
  constructor
  · unfold fence_on_sync_fail; rfl
  · unfold wal_commit_plan
    unfold fence_on_sync_fail
    rfl

/-- AS-IS dente: Apply/Ok even after a failed required sync. -/
theorem wal_commit_plan_as_is_dente :
    wal_commit_plan_as_is true true = ok WalCommitPlan.AppendSyncApplyOk := by
  unfold wal_commit_plan_as_is
  rfl

/-- RFC-0191 P1.2 D1-script: the whole Bool×Bool space of the plan rustc
links (`commit_ops_with` matches it). Required sync that succeeded is
Sync before Apply/Ok; required sync that failed is Fence (never
Apply/Ok); no required sync is Apply/Ok without Sync. Concrete
`wal_commit_plan true false` does **not** pay this — the binder covers
the space. -/
theorem d1_wal_commit_plan :
    ∀ (need_sync sync_fail : Bool),
      wal_commit_plan need_sync sync_fail
        = ok (if need_sync then
                (if sync_fail then WalCommitPlan.AppendSyncFence
                 else WalCommitPlan.AppendSyncApplyOk)
              else WalCommitPlan.AppendApplyOk) := by
  intro need_sync sync_fail
  unfold wal_commit_plan fence_on_sync_fail
  cases need_sync <;> cases sync_fail <;> rfl

/-- `put_if_absent`: no live key ⇒ put. -/
theorem cas_absent_put_empty_puts :
    cas_absent_put false = ok true := by
  unfold cas_absent_put
  rfl

/-- Live key ⇒ do not put (CasMismatch). -/
theorem cas_absent_put_live_refuses :
    cas_absent_put true = ok false := by
  unfold cas_absent_put
  rfl

/-- AS-IS dente: live key still puts. -/
theorem cas_absent_put_as_is_dente :
    cas_absent_put_as_is true = ok true := by
  unfold cas_absent_put_as_is
  rfl

/-- RFC-0191 P2.3 (twenty-ninth if): the empty-batch gate returns ok v
    exactly when v equals the machine comparison of the batch length
    against zero — the computation rule of the do-block body (the body
    holds no monadic step, so no disposition can hide behind a bind). -/
theorem batch_is_empty_ok_iff_zero :
    ∀ (n : U64) (v : Bool),
      (batch_is_empty n = ok v) ↔ ((n = 0#u64 : Bool) = v) := by
  intro n v
  unfold batch_is_empty
  constructor
  · intro h
    injection h with _
  · intro h
    rw [h]

/-- RFC-0191 P2.3 (thirtieth if): the directory-sync gate returns ok v
    exactly when v is the sync flag itself — the computation rule of the
    do-block body (a pure lift; rename/create is followed by a dir fsync
    precisely when open-options sync is on). -/
theorem dir_sync_required_ok_iff_sync :
    ∀ (sync v : Bool),
      (dir_sync_required sync = ok v) ↔ (sync = v) := by
  intro sync v
  unfold dir_sync_required
  constructor
  · intro h
    injection h with _
  · intro h
    rw [h]

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

/-- RFC-0198 P0.1 (first registered glue close): the commit plan is ok v
    exactly along the fence chain — the callee `fence_on_sync_fail` lands
    ok on some b, and the plan's own two ifs route b/need_sync to the
    plan value. The iff is the computation rule over BOTH extracted
    bodies (plan and callee): required sync that failed is Fence; required
    sync that succeeded is Sync-Apply-Ok; no required sync is Apply-Ok. -/
theorem wal_commit_plan_ok_iff_fence_chain :
    ∀ (need_sync sync_failed : Bool) (v : WalCommitPlan),
      (wal_commit_plan need_sync sync_failed = ok v) ↔
        (∃ b, fence_on_sync_fail need_sync sync_failed = ok b ∧
          ((b = true ∧ v = WalCommitPlan.AppendSyncFence) ∨
            (¬(b = true) ∧ need_sync = true ∧
              v = WalCommitPlan.AppendSyncApplyOk) ∨
            (¬(b = true) ∧ ¬(need_sync = true) ∧
              v = WalCommitPlan.AppendApplyOk))) := by
  intro need_sync sync_failed v
  unfold wal_commit_plan
  constructor
  · intro hval
    obtain ⟨b, hw, hval⟩ := bind_ok_inv _ _ _ hval
    refine ⟨b, hw, ?_⟩
    split at hval
    · next hb =>
      injection hval with hv
      exact Or.inl ⟨hb, hv.symm⟩
    · next hb =>
      split at hval
      · next hns =>
        injection hval with hv
        exact Or.inr (Or.inl ⟨hb, hns, hv.symm⟩)
      · next hns =>
        injection hval with hv
        exact Or.inr (Or.inr ⟨hb, hns, hv.symm⟩)
  · rintro ⟨b, hw, hb | ⟨hb, hns, hv⟩ | ⟨hb, hns, hv⟩⟩
    · refine bind_intro b hw ?_
      rw [if_pos hb.1, hb.2]
    · refine bind_intro b hw ?_
      rw [if_neg hb, if_pos hns, hv]
    · refine bind_intro b hw ?_
      rw [if_neg hb, if_neg hns, hv]

/-- `put_if_eq`: live == expected ⇒ put. -/
theorem cas_eq_put_match_puts :
    cas_eq_put true = ok true := by
  unfold cas_eq_put
  rfl

/-- Mismatch ⇒ do not put (CasMismatch). -/
theorem cas_eq_put_mismatch_refuses :
    cas_eq_put false = ok false := by
  unfold cas_eq_put
  rfl

/-- AS-IS dente: mismatch still puts. -/
theorem cas_eq_put_as_is_dente :
    cas_eq_put_as_is false = ok true := by
  unfold cas_eq_put_as_is
  rfl

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:write_admission`):
    the write-admission idle gate is true EXACTLY when every stall
    knob is off — fate forall over the extracted body (RFC-0170
    P2.4); the AS-IS mutant answers idle with knobs armed (the lie
    the DST plant `write_admission_idle_on_live_stall_is_not_ok`
    refutes). -/
theorem write_admission_idle_fate_iff :
    ∀ (mem_stall pressure_l0 stall_l0 v : Bool),
      (write_admission_idle mem_stall pressure_l0 stall_l0 = ok v) ↔
        ((v = true ∧ ¬mem_stall ∧ ¬pressure_l0 ∧ ¬stall_l0)
          ∨ (v = false ∧ (mem_stall ∨ pressure_l0 ∨ stall_l0))) := by
  intro mem_stall pressure_l0 stall_l0 v
  unfold write_admission_idle
  cases mem_stall <;> cases pressure_l0 <;> cases stall_l0 <;> cases v <;> simp

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:write_admit`):
    the hard-admit verdict is StallMem EXACTLY when the armed mem
    axis is over its limit, StallL0 exactly when mem passed but the
    armed L0 axis is over, and Ok exactly when neither axis stalls —
    fate forall over the extracted body (RFC-0170 P2.4); the AS-IS
    mutant always admits (the lie the DST plant
    `write_admit_on_live_mem_over_is_not_ok` refutes). -/
theorem write_admit_fate_iff :
    ∀ (mem_bytes : U64) (mem_armed : Bool) (mem_limit l0 : U64)
      (l0_armed : Bool) (l0_limit : U64) (r : WriteAdmit),
      (write_admit mem_bytes mem_armed mem_limit l0 l0_armed l0_limit = ok r) ↔
        ((r = WriteAdmit.StallMem ∧ mem_armed = true ∧ mem_bytes >= mem_limit)
          ∨ (r = WriteAdmit.StallL0 ∧ l0_armed = true ∧ l0 >= l0_limit
              ∧ ¬ (mem_armed = true ∧ mem_bytes >= mem_limit))
          ∨ (r = WriteAdmit.Ok
              ∧ ¬ (mem_armed = true ∧ mem_bytes >= mem_limit)
              ∧ ¬ (l0_armed = true ∧ l0 >= l0_limit))) := by
  intro mem_bytes mem_armed mem_limit l0 l0_armed l0_limit r
  unfold write_admit
  cases mem_armed <;> cases l0_armed <;>
    by_cases hmem : mem_bytes >= mem_limit <;>
    by_cases hl0 : l0 >= l0_limit <;> simp [hmem, hl0] <;> exact eq_comm

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:seq_exhausted`):
    the sequence counter is exhausted EXACTLY when it has burned past
    the ceiling — fate forall over the extracted body (RFC-0170 P2.4);
    the AS-IS mutant never reports exhaustion (wrap / burn past the
    ceiling — the lie the DST plant
    `seq_exhausted_on_live_ceiling_is_not_ok` refutes). -/
theorem seq_exhausted_fate_iff :
    ∀ (seq max : U64) (v : Bool),
      (seq_exhausted seq max = ok v) ↔ v = decide (seq > max) := by
  intro seq max v
  unfold seq_exhausted
  simp
  exact eq_comm

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:fence_on_sync_fail`):
    the fence trips EXACTLY when a sync was required and that sync
    failed — fate forall over the extracted body (RFC-0170 P2.4);
    the AS-IS mutant never fences (the lie the DST plant
    `fence_on_sync_fail_on_live_required_fail_is_not_ok` refutes). -/
theorem fence_on_sync_fail_fate_iff :
    ∀ (sync_required sync_failed v : Bool),
      (fence_on_sync_fail sync_required sync_failed = ok v) ↔
        v = (sync_required && sync_failed) := by
  intro sync_required sync_failed v
  unfold fence_on_sync_fail
  cases sync_required <;> cases sync_failed <;> cases v <;> simp

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:wal_commit_plan`):
    the WAL append plan is AppendSyncFence EXACTLY when sync was
    needed and failed, AppendSyncApplyOk EXACTLY when sync was
    needed and succeeded, and AppendApplyOk EXACTLY when no sync was
    needed — fate forall over the extracted body (RFC-0170 P2.4);
    the AS-IS mutant returns the wrong plan (the lie the DST plant
    `wal_commit_plan_on_live_sync_fail_is_not_ok` refutes). -/
theorem wal_commit_plan_fate_iff :
    ∀ (need_sync sync_failed : Bool) (r : WalCommitPlan),
      (wal_commit_plan need_sync sync_failed = ok r) ↔
        ((r = WalCommitPlan.AppendSyncFence
            ∧ need_sync = true ∧ sync_failed = true)
          ∨ (r = WalCommitPlan.AppendSyncApplyOk
            ∧ need_sync = true ∧ sync_failed = false)
          ∨ (r = WalCommitPlan.AppendApplyOk ∧ need_sync = false)) := by
  intro need_sync sync_failed r
  unfold wal_commit_plan fence_on_sync_fail
  cases need_sync <;> cases sync_failed <;> simp <;> exact eq_comm

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:torn_head_empty_log`):
    a torn head counts as an empty log EXACTLY when the length is
    below the tiny-log bound — fate forall over the extracted body
    (RFC-0170 P2.4); the AS-IS mutant calls every head empty (the
    lie the DST plant `torn_head_is_empty_log_on_live_large_wal_is_not_ok`
    refutes). -/
theorem torn_head_empty_log_fate_iff :
    ∀ (len tiny_max : U64) (v : Bool),
      (torn_head_is_empty_log len tiny_max = ok v) ↔
        v = decide (len < tiny_max) := by
  intro len tiny_max v
  unfold torn_head_is_empty_log
  simp
  exact eq_comm

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:torn_tail_needs_cut`):
    a torn tail needs the cut EXACTLY when the length overhangs the
    last good offset — fate forall over the extracted body
    (RFC-0170 P2.4); the AS-IS mutant never cuts (the lie the DST
    plant `torn_tail_needs_cut_on_live_overhang_is_not_ok` refutes). -/
theorem torn_tail_needs_cut_fate_iff :
    ∀ (len last_good : U64) (v : Bool),
      (torn_tail_needs_cut len last_good = ok v) ↔
        v = decide (len > last_good) := by
  intro len last_good v
  unfold torn_tail_needs_cut
  simp
  exact eq_comm

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:seq_after_feed`):
    a sequence is after the feed EXACTLY when it overhangs the feed
    ceiling — fate forall over the extracted body (RFC-0170 P2.4);
    the AS-IS mutant never sees past the feed (the lie the DST
    plant `seq_after_feed_on_live_newer_is_not_ok` refutes). -/
theorem seq_after_feed_fate_iff :
    ∀ (seq feed_max : U64) (v : Bool),
      (seq_after_feed seq feed_max = ok v) ↔
        v = decide (seq > feed_max) := by
  intro seq feed_max v
  unfold seq_after_feed
  simp
  exact eq_comm

/-- RFC-0213 P0.1 (storage cadence, atom `catalog:pit_resync_rewrite`):
    a point-in-time resync needs the rewrite EXACTLY when the entry
    is a resync — fate forall over the extracted body (RFC-0170
    P2.4); the AS-IS mutant skips the rewrite (the lie the DST plant
    `pit_resync_needs_rewrite_on_live_resync_is_not_ok` refutes). -/
theorem pit_resync_rewrite_fate_iff :
    ∀ (is_resync v : Bool),
      (pit_resync_needs_rewrite is_resync = ok v) ↔ v = is_resync := by
  intro is_resync v
  unfold pit_resync_needs_rewrite
  cases is_resync <;> cases v <;> simp

/-- RFC-0219 P1.1 (átomo `catalog:dir_sync_plan`): o dir-fsync pós-rename
    (SST `.tmp`, chunk fundido, portão dir do DB) é pago EXATAMENTE em
    modo sync — o dentry do rename é durável antes de voltar; async
    pula (recuperação tolera dentry de nome-tmp sumiu). O AS-IS nunca
    paga (dentry some pós-crash mesmo em sync — dente plantado). -/
theorem dir_sync_plan_fate_iff :
    ∀ (sync : Bool) (plan : DirSyncPlan),
      (dir_sync_plan sync = ok plan) ↔
        ((sync = true ∧ plan = DirSyncPlan.SyncDirNow) ∨
          (sync = false ∧ plan = DirSyncPlan.SkipDirSync)) := by
  intro sync plan
  unfold dir_sync_plan dir_sync_required
  cases sync <;> simp_all <;> exact eq_comm

/-- RFC-0219 P1.2 (átomo `catalog:fence_admission`): um Db com fence de
    durabilidade recusa cada nova operação EXATAMENTE quando o fence
    está armado — fail-closed; sem fence admite. O AS-IS admite sempre
    (barreira falhada segue servindo escrita como se durável — dente
    plantado). -/
theorem fence_admission_plan_fate_iff :
    ∀ (fenced : Bool) (plan : FenceAdmission),
      (fence_admission_plan fenced = ok plan) ↔
        ((fenced = true ∧ plan = FenceAdmission.RefuseFenced) ∨
          (fenced = false ∧ plan = FenceAdmission.AdmitOps)) := by
  intro fenced plan
  unfold fence_admission_plan
  cases fenced <;> simp_all <;> exact eq_comm

/-- RFC-0219 P1.2 (átomo `catalog:fence_record`): só o PRIMEIRO fence
    registra o relatório da janela incerta — fence posterior mantém o
    primeiro (o mais largo, o honesto). O AS-IS re-registra (encolhe a
    janela que o client sabe estar não-provada — dente plantado). -/
theorem fence_record_plan_fate_iff :
    ∀ (has_report : Bool) (plan : FenceRecordPlan),
      (fence_record_plan has_report = ok plan) ↔
        ((has_report = true ∧ plan = FenceRecordPlan.KeepExisting) ∨
          (has_report = false ∧ plan = FenceRecordPlan.RecordFirst)) := by
  intro has_report plan
  unfold fence_record_plan
  cases has_report <;> simp_all <;> exact eq_comm

/-- RFC-0219 P1.2 (átomo `catalog:group_batch_sync`): um batch com flag
    de sync EXATAMENTE força a barreira única do grupo (one fsync
    compartilhado); batch async apenas viaja no agregado. O AS-IS deixa
    tudo viajar (client que pediu sync é ackado sem barreira — dente
    plantado). -/
theorem group_batch_sync_plan_fate_iff :
    ∀ (client_sync : Bool) (plan : GroupSyncPlan),
      (group_batch_sync_plan client_sync = ok plan) ↔
        ((client_sync = true ∧ plan = GroupSyncPlan.BatchForcesSync) ∨
          (client_sync = false ∧ plan = GroupSyncPlan.BatchRidesGroup)) := by
  intro client_sync plan
  unfold group_batch_sync_plan
  cases client_sync <;> simp_all <;> exact eq_comm

/-- RFC-0219 P1.4 (átomo `catalog:pit_resync_rewrite`): o open reescreve
    o WAL a partir do prefixo recuperado EXATAMENTE quando o relatório
    de recuperação é um resync; sem resync o prefixo fica no disco
    como-is. O AS-IS nunca reescreve (o dano mid-log sobrevive ao
    próximo open fail-closed — dente plantado). -/
theorem pit_resync_rewrite_plan_fate_iff :
    ∀ (is_resync : Bool) (plan : PitResyncRewritePlan),
      (pit_resync_rewrite_plan is_resync = ok plan) ↔
        ((is_resync = true ∧
            plan = PitResyncRewritePlan.RewriteWalFromPrefix) ∨
          (is_resync = false ∧
            plan = PitResyncRewritePlan.KeepRecoveredPrefix)) := by
  intro is_resync plan
  unfold pit_resync_rewrite_plan pit_resync_needs_rewrite
  cases is_resync <;> simp_all <;> exact eq_comm

/-- RFC-0219 P2.1 (átomo `catalog:parked_pop_plan`): o pop da fila
    estacionada acontece EXATAMENTE quando a fila está não-vazia; fila
    vazia não entrega nada ao fold. O AS-IS popa da fila vazia (índice
    de frente no nada — dente plantado). -/
theorem parked_pop_plan_fate_iff :
    ∀ (parked_len : U64) (plan : ParkedPopPlan),
      (parked_pop_plan parked_len = ok plan) ↔
        (((parked_len = 0#u64 : Bool) = true ∧
            plan = ParkedPopPlan.NoParkedTables) ∨
          ((parked_len = 0#u64 : Bool) = false ∧
            plan = ParkedPopPlan.PopOldestParked)) := by
  intro parked_len plan
  simp only [parked_pop_plan]
  constructor
  · intro hval
    obtain ⟨b, hw, hm⟩ := bind_ok_inv _ _ _ hval
    rw [batch_is_empty_ok_iff_zero] at hw
    cases b with
    | true =>
        simp at hm
        subst hm
        exact Or.inl ⟨hw, rfl⟩
    | false =>
        simp at hm
        subst hm
        exact Or.inr ⟨hw, rfl⟩
  · rintro (⟨hz, hplan⟩ | ⟨hz, hplan⟩)
    · refine bind_intro true ?_ ?_
      · rw [batch_is_empty_ok_iff_zero]
        exact hz
      · simp [hplan]
    · refine bind_intro false ?_ ?_
      · rw [batch_is_empty_ok_iff_zero]
        exact hz
      · simp [hplan]
