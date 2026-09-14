-- RFC-0222 P2.1 / RFC-0220 P0.2 — writer-spine dual-unfold:
-- flusher_gate_plan × parked_debt_plan.
-- RFC-0224 P0.1 / RFC-0220 P0.3 — changelog_durable_commit_fate ×
-- wal_commit_plan (Count+sync ⇒ AppendSync; Skip async ⇒ AppendApplyOk).
-- RFC-0224 P0.2 / RFC-0220 P0.4 — wal_commit_plan::AppendSyncFence ×
-- fence_admission_plan (sync fail ⇒ RefuseFenced).
--
-- Sem worker (Workerless) NADA parqueia — inclusive com dívida no cap.
-- O AS-IS diz WorkerDrains sempre (writer workerless dorme para sempre).
-- Cada perna é o átomo iff já registrado; os corpos extraídos não abrem.
import Aeneas
import Flush
import Changelog
import WriteAdmission
open Aeneas.Std Result
open pedra_aeneas_flush_kernel
open pedra_aeneas_changelog_kernel
open pedra_aeneas_write_admission_kernel

/-- RFC-0222 P2.1: a cadeia workerless COMPOSTA — o gate é Workerless
    EXATAMENTE quando nenhum worker está attached, e a dívida parked é
    DebtAtCap EXATAMENTE no/acima do cap. Dual-unfold dos dois átomos
    (`catalog:flusher_gate_plan`, `catalog:parked_debt_plan`). Sem worker
    o plano NÃO vira WorkerDrains mesmo com dívida no cap. -/
theorem writer_workerless_gate_and_debt_iff :
    ∀ (attached : Bool) (parked cap : U64)
      (g : FlusherGate) (d : ParkedDebtPlan),
      (flusher_gate_plan attached = ok g ∧
          parked_debt_plan parked cap = ok d) ↔
        (((attached = true ∧ g = FlusherGate.WorkerDrains) ∨
            (attached = false ∧ g = FlusherGate.Workerless)) ∧
          ((parked < cap ∧ d = ParkedDebtPlan.NoDebtBelowCap) ∨
            (¬(parked < cap) ∧ d = ParkedDebtPlan.DebtAtCap))) := by
  intro attached parked cap g d
  constructor
  · intro ⟨hg, hd⟩
    exact ⟨(flusher_gate_plan_fate_iff attached g).mp hg,
      (parked_debt_plan_fate_iff parked cap d).mp hd⟩
  · intro ⟨hg, hd⟩
    exact ⟨(flusher_gate_plan_fate_iff attached g).mpr hg,
      (parked_debt_plan_fate_iff parked cap d).mpr hd⟩

/-- Sem worker, o gate composto nunca é WorkerDrains — mesmo com
    parked ≥ cap (a dívida existe, mas ninguém a drena). -/
theorem workerless_never_drains :
    ∀ (parked cap : U64) (g : FlusherGate) (d : ParkedDebtPlan),
      (flusher_gate_plan false = ok g ∧
          parked_debt_plan parked cap = ok d) →
        g = FlusherGate.Workerless := by
  intro parked cap g d h
  have := (writer_workerless_gate_and_debt_iff false parked cap g d).mp h
  rcases this.1 with ⟨htrue, _⟩ | ⟨_, hg⟩
  · cases htrue
  · exact hg

/-- AS-IS dente: o gate ignora `attached` e sempre WorkerDrains —
    writer workerless dorme para sempre (RFC-0219 P2.2 plant). -/
theorem flusher_gate_plan_as_is_always_drains :
    ∀ (attached : Bool),
      flusher_gate_plan_as_is attached = ok FlusherGate.WorkerDrains := by
  intro attached
  unfold flusher_gate_plan_as_is
  rfl

/-- RFC-0224 P0.1 / RFC-0220 P0.3: a cadeia do sync COMPOSTA — o
    changelog Count EXATAMENTE na resolução de sync do cliente/DB, e o
    plano WAL é AppendSyncFence / AppendSyncApplyOk / AppendApplyOk
    EXATAMENTE no trio (need_sync, sync_failed). Dual-unfold dos dois
    átomos (`catalog:changelog_durable_commit`, `catalog:wal_commit_plan`).
    Corpos extraídos não abrem. -/
theorem writer_sync_chain_iff :
    ∀ (client_set client_sync db_sync need_sync sync_failed : Bool)
      (c : ChangelogCommitFate) (w : WalCommitPlan),
      (changelog_durable_commit_fate client_set client_sync db_sync = ok c ∧
          wal_commit_plan need_sync sync_failed = ok w) ↔
        (((client_set = true ∧ client_sync = true ∧
              c = ChangelogCommitFate.Count) ∨
            (client_set = true ∧ client_sync = false ∧
              c = ChangelogCommitFate.Skip) ∨
            (client_set = false ∧ db_sync = true ∧
              c = ChangelogCommitFate.Count) ∨
            (client_set = false ∧ db_sync = false ∧
              c = ChangelogCommitFate.Skip)) ∧
          ((w = WalCommitPlan.AppendSyncFence ∧
              need_sync = true ∧ sync_failed = true) ∨
            (w = WalCommitPlan.AppendSyncApplyOk ∧
              need_sync = true ∧ sync_failed = false) ∨
            (w = WalCommitPlan.AppendApplyOk ∧ need_sync = false))) := by
  intro client_set client_sync db_sync need_sync sync_failed c w
  constructor
  · intro ⟨hc, hw⟩
    exact ⟨(changelog_durable_commit_fate_fate_iff
              client_set client_sync db_sync c).mp hc,
      (wal_commit_plan_fate_iff need_sync sync_failed w).mp hw⟩
  · intro ⟨hc, hw⟩
    exact ⟨(changelog_durable_commit_fate_fate_iff
              client_set client_sync db_sync c).mpr hc,
      (wal_commit_plan_fate_iff need_sync sync_failed w).mpr hw⟩

/-- Count + sync requerido ⇒ o plano WAL é AppendSync (Fence ou ApplyOk).
    Skip sozinho não basta: o RFC exige o elo Count **com** sync. -/
theorem count_with_sync_requires_append_sync :
    ∀ (client_set client_sync db_sync sync_failed : Bool)
      (c : ChangelogCommitFate) (w : WalCommitPlan),
      (changelog_durable_commit_fate client_set client_sync db_sync = ok c ∧
          wal_commit_plan true sync_failed = ok w) →
        c = ChangelogCommitFate.Count →
        (w = WalCommitPlan.AppendSyncFence ∨
          w = WalCommitPlan.AppendSyncApplyOk) := by
  intro client_set client_sync db_sync sync_failed c w h hc
  have := (writer_sync_chain_iff client_set client_sync db_sync
    true sync_failed c w).mp h
  rcases this.2 with hfence | hok | happly
  · exact Or.inl hfence.1
  · exact Or.inr hok.1
  · cases happly.2

/-- Skip async ⇒ WAL é AppendApplyOk (sem Sync, sem Fence). -/
theorem skip_async_is_append_apply_ok :
    ∀ (client_set client_sync db_sync : Bool)
      (c : ChangelogCommitFate) (w : WalCommitPlan),
      (changelog_durable_commit_fate client_set client_sync db_sync = ok c ∧
          wal_commit_plan false false = ok w) →
        c = ChangelogCommitFate.Skip →
        w = WalCommitPlan.AppendApplyOk := by
  intro client_set client_sync db_sync c w h hc
  have := (writer_sync_chain_iff client_set client_sync db_sync
    false false c w).mp h
  rcases this.2 with hfence | hok | happly
  · cases hfence.2.1
  · cases hok.2.1
  · exact happly.1

/-- AS-IS dente composto: changelog nunca conta (Skip em todo input) e
    o WAL nunca cerca (Apply/Ok mesmo com sync requerido falho). -/
theorem writer_sync_as_is_never_counts_and_never_fences :
    changelog_durable_commit_fate_as_is true true true
        = ok ChangelogCommitFate.Skip ∧
      wal_commit_plan_as_is true true
        = ok WalCommitPlan.AppendSyncApplyOk := by
  constructor
  · unfold changelog_durable_commit_fate_as_is
    rfl
  · unfold wal_commit_plan_as_is
    rfl

/-- RFC-0224 P0.2 / RFC-0220 P0.4: a cadeia do fence COMPOSTA — o plano
    WAL é AppendSyncFence EXATAMENTE quando sync requerido falhou, e a
    admissão é RefuseFenced EXATAMENTE com o fence armado. Dual-unfold
    de `wal_commit_plan_fate_iff` × `fence_admission_plan_fate_iff`.
    Corpos extraídos não abrem. -/
theorem writer_fence_chain_iff :
    ∀ (need_sync sync_failed fenced : Bool)
      (w : WalCommitPlan) (a : FenceAdmission),
      (wal_commit_plan need_sync sync_failed = ok w ∧
          fence_admission_plan fenced = ok a) ↔
        (((w = WalCommitPlan.AppendSyncFence ∧
              need_sync = true ∧ sync_failed = true) ∨
            (w = WalCommitPlan.AppendSyncApplyOk ∧
              need_sync = true ∧ sync_failed = false) ∨
            (w = WalCommitPlan.AppendApplyOk ∧ need_sync = false)) ∧
          ((fenced = true ∧ a = FenceAdmission.RefuseFenced) ∨
            (fenced = false ∧ a = FenceAdmission.AdmitOps))) := by
  intro need_sync sync_failed fenced w a
  constructor
  · intro ⟨hw, ha⟩
    exact ⟨(wal_commit_plan_fate_iff need_sync sync_failed w).mp hw,
      (fence_admission_plan_fate_iff fenced a).mp ha⟩
  · intro ⟨hw, ha⟩
    exact ⟨(wal_commit_plan_fate_iff need_sync sync_failed w).mpr hw,
      (fence_admission_plan_fate_iff fenced a).mpr ha⟩

/-- Sync requerido falhou ⇒ WAL é AppendSyncFence e, com o fence armado,
    a admissão recusa TUDO depois. -/
theorem append_sync_fence_refuses_all_after :
    ∀ (a : FenceAdmission),
      (wal_commit_plan true true = ok WalCommitPlan.AppendSyncFence ∧
          fence_admission_plan true = ok a) →
        a = FenceAdmission.RefuseFenced := by
  intro a h
  have := (writer_fence_chain_iff true true true
    WalCommitPlan.AppendSyncFence a).mp h
  rcases this.2 with hrefuse | hadmit
  · exact hrefuse.2
  · cases hadmit.1

/-- AS-IS dente composto: WAL nunca cerca (Apply/Ok no sync falho) e a
    admissão admite mesmo com fence armado. -/
theorem writer_fence_as_is_admits :
    wal_commit_plan_as_is true true
        = ok WalCommitPlan.AppendSyncApplyOk ∧
      fence_admission_plan_as_is true = ok FenceAdmission.AdmitOps := by
  constructor
  · unfold wal_commit_plan_as_is
    rfl
  · unfold fence_admission_plan_as_is
    rfl
