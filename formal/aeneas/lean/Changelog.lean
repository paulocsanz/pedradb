-- Theorems over Aeneas extract of changelog_kernel.rs
import Aeneas
import ChangelogKernel
open Aeneas.Std Result
open pedra_aeneas_changelog_kernel

theorem changelog_should_store_due :
    changelog_should_store 5#u64 3#u64 = ok true := by
  unfold changelog_should_store
  have hgt : (3#u64 > 0#u64) = true := by native_decide
  have hge : (5#u64 ≥ 3#u64) = true := by native_decide
  simp [hgt, hge]
/-- RFC-0218 P0.3 1/6 (atom `catalog:changelog`): the decision of
    rebuild is EXACTLY the cited window — feed empty with seq > 0
    needs of rebuild; feed live never (the feed is the truth). The AS-IS
    always returns false (blind rebuild — tooth planted in the model
    Stateright do fn real). -/
theorem changelog_needs_sst_rebuild_fate_iff :
    ∀ (feed_empty : Bool) (last_sequence : U64) (v : Bool),
      (changelog_needs_sst_rebuild feed_empty last_sequence = ok v) ↔
        ((feed_empty = true ∧ v = (decide (last_sequence > 0#u64) : Bool)) ∨
          (feed_empty = false ∧ v = false)) := by
  intro feed_empty last_sequence v
  cases feed_empty with
  | true =>
    constructor
    · intro hval
      simp only [changelog_needs_sst_rebuild] at hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2)
      · subst hv
        rfl
      · exact absurd h2.1 (fun h => Bool.noConfusion h)
  | false =>
    constructor
    · intro hval
      simp only [changelog_needs_sst_rebuild] at hval
      injection hval with hv
      exact Or.inr ⟨rfl, hv.symm⟩
    · rintro (h1 | ⟨-, hv⟩)
      · exact absurd h1.1 (fun h => Bool.noConfusion h)
      · subst hv
        rfl
/-- RFC-0218 P0.3 2/6 (atom `catalog:changelog_should_store`): the
    debounce is EXACTLY the gate cited — interval 0 never
    stores on the commit path; a positive interval stores when the
    commits since the last reach the interval. The AS-IS stores the
    each commit (ignora the interval — tooth planted). -/
theorem changelog_should_store_fate_iff :
    ∀ (commits_since : U64) (interval : U64) (v : Bool),
      (changelog_should_store commits_since interval = ok v) ↔
        ((interval > 0#u64 ∧
            v = decide (commits_since ≥ interval)) ∨
          (¬(interval > 0#u64) ∧ v = false)) := by
  intro commits_since interval v
  constructor
  · intro hval
    simp only [changelog_should_store] at hval
    split at hval
    · next hg =>
      exact Or.inl ⟨hg, by injection hval with hv; exact hv.symm⟩
    · next hg =>
      exact Or.inr ⟨hg, by injection hval with hv; exact hv.symm⟩
  · rintro (⟨hg, hv⟩ | ⟨hg, hv⟩)
    · simp only [changelog_should_store]
      rw [if_pos hg]
      subst hv
      rfl
    · simp only [changelog_should_store]
      rw [if_neg hg]
      subst hv
      rfl
/-- RFC-0218 P0.3 3/6 (atom `catalog:changelog_budget`): the
    budget of rebuild is EXACTLY the cited comparison —
    materializar cabe in the budget iff live_entries ≤ budget_entries.
    The AS-IS always returns true (materialization without restraint — tooth
    planted). -/
theorem changelog_rebuild_within_budget_fate_iff :
    ∀ (live_entries : U64) (budget_entries : U64) (v : Bool),
      (changelog_rebuild_within_budget live_entries budget_entries = ok v) ↔
        (v = decide (live_entries ≤ budget_entries)) := by
  intro live_entries budget_entries v
  constructor
  · intro hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl
/-- RFC-0219 P0.1 (atom `catalog:changelog_durable_commit`): the destination
    of the debounce of CHANGELOG in a finished commit is EXACTLY the resolution
    of sync — counts iff the client asked for sync or, without a client flag, the
    DB default syncs; everything else skips (the cache delays and the
    reopen rebuilds the feed of the WAL — RFC-0019). The AS-IS never counts:
    every crash pays the replay integral of the WAL (tooth planted in the kernel). -/
theorem changelog_durable_commit_fate_fate_iff :
    ∀ (client_set : Bool) (client_sync : Bool) (db_sync : Bool)
      (v : ChangelogCommitFate),
      (changelog_durable_commit_fate client_set client_sync db_sync = ok v) ↔
        ((client_set = true ∧ client_sync = true ∧
            v = ChangelogCommitFate.Count) ∨
          (client_set = true ∧ client_sync = false ∧
            v = ChangelogCommitFate.Skip) ∨
          (client_set = false ∧ db_sync = true ∧
            v = ChangelogCommitFate.Count) ∨
          (client_set = false ∧ db_sync = false ∧
            v = ChangelogCommitFate.Skip)) := by
  intro client_set client_sync db_sync v
  simp only [changelog_durable_commit_fate]
  split <;> rename_i c
  · split <;> rename_i c2
    · constructor
      · intro hval
        injection hval with hv
        exact Or.inl ⟨c, c2, hv.symm⟩
      · rintro (⟨-, -, hv⟩ | h2 | h3 | h4)
        · subst hv
          rfl
        · exact absurd h2.2.1 (by simp [*])
        · exact absurd h3.1 (by simp [*])
        · exact absurd h4.1 (by simp [*])
    · rw [Bool.not_eq_true] at c2
      constructor
      · intro hval
        injection hval with hv
        exact Or.inr (Or.inl ⟨c, c2, hv.symm⟩)
      · rintro (h1 | ⟨-, -, hv⟩ | h3 | h4)
        · exact absurd h1.2.1 (by simp [*])
        · subst hv
          rfl
        · exact absurd h3.1 (by simp [*])
        · exact absurd h4.1 (by simp [*])
  · rw [Bool.not_eq_true] at c
    split <;> rename_i c2
    · constructor
      · intro hval
        injection hval with hv
        exact Or.inr (Or.inr (Or.inl ⟨c, c2, hv.symm⟩))
      · rintro (h1 | h2 | ⟨-, -, hv⟩ | h4)
        · exact absurd h1.1 (by simp [*])
        · exact absurd h2.1 (by simp [*])
        · subst hv
          rfl
        · exact absurd h4.2.1 (by simp [*])
    · rw [Bool.not_eq_true] at c2
      constructor
      · intro hval
        injection hval with hv
        exact Or.inr (Or.inr (Or.inr ⟨c, c2, hv.symm⟩))
      · rintro (h1 | h2 | h3 | ⟨-, -, hv⟩)
        · exact absurd h1.1 (by simp [*])
        · exact absurd h2.1 (by simp [*])
        · exact absurd h3.2.1 (by simp [*])
        · subst hv
          rfl
/-- RFC-0219 P0.2 (atom `catalog:wal_archive_delete`): the destination of the
    chain arquivada is EXACTLY the cited comparison — while the
    publish do MANIFEST atrasa os files (segments above de
    manifest_published_seq are the single durable copy of the window), guard;
    publish covering the chain, releases the delete. The AS-IS deleta the window
    unpublished (tooth planted in the kernel). -/
theorem wal_archive_delete_plan_fate_iff :
    ∀ (manifest_published_seq : U64) (wal_archive_max_seq : U64)
      (v : WalArchiveDelete),
      (wal_archive_delete_plan manifest_published_seq wal_archive_max_seq
          = ok v) ↔
        ((manifest_published_seq < wal_archive_max_seq ∧
            v = WalArchiveDelete.KeepUntilPublished) ∨
          (¬(manifest_published_seq < wal_archive_max_seq) ∧
            v = WalArchiveDelete.DeleteCovered)) := by
  intro manifest_published_seq wal_archive_max_seq v
  simp only [wal_archive_delete_plan]
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

/-- RFC-0219 P1.4 (atom `catalog:changelog_store_plan`): the store point
    synchronous writes the feed EXACTLY when the publish durable of the
    MANIFEST covered the window arquivada; publish failed holds the store
    — the segments archived are the single copy durable of the window. O
    AS-IS writes with publish failed (the store deletes segments that
    none MANIFEST publicado covers — tooth planted). -/
theorem changelog_store_plan_fate_iff :
    ∀ (publish_ok : Bool) (plan : ChangelogStorePlan),
      (changelog_store_plan publish_ok = ok plan) ↔
        ((publish_ok = true ∧
            plan = ChangelogStorePlan.StoreFeed) ∨
          (publish_ok = false ∧
            plan = ChangelogStorePlan.SkipStorePublishHolds)) := by
  intro publish_ok plan
  unfold changelog_store_plan
  cases publish_ok <;> simp_all <;> exact eq_comm
