-- Cross-lib: the queued FINISH protocol (RFC-0212 P2.2) — the
-- discard/persist chain COMPOSED over the four registered atoms
-- (`catalog:discard_leader`, `catalog:discard_uncommitted`,
-- `catalog:persist_fence`, `catalog:persist_hist`).
-- Registration rule: a row needs a single catalog pair/entry; this
-- composition spans four kernels — same reason the other compose
-- libs carry no row (reason dated in findings).
import Aeneas
import Membership
open Aeneas.Std Result
open pedra_aeneas_membership_kernel

/-- Every registered finish gate is the identity on its local
    input — derived from the registered iff atoms, not from the
    bodies. -/
private theorem gate_of_iff (x : Bool) (g : Bool → Result Bool)
    (hiff : ∀ v : Bool, (g x = ok v) ↔
      ((v = true ∧ x = true) ∨ (v = false ∧ x = false))) :
    g x = ok x := by
  cases x with
  | true  => exact (hiff true).mpr  (Or.inl ⟨rfl, rfl⟩)
  | false => exact (hiff false).mpr (Or.inr ⟨rfl, rfl⟩)

/-- RFC-0212 P2.2: the queued finish chain — discard-leader local,
    then the discard counts, then the abort fence persists, then the
    hist persists, each bound on the previous atom's output — lands
    `ok v` with `v` EXACTLY the node's locality, for EVERY `in_ids`:
    the fence/hist legs fire on a local replica already removed from
    `ids` (the 0136 semantics survive the composition). -/
theorem queued_finish_chain_fate :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (Aeneas.Std.bind (discard_leader_local is_local)
          (fun dl => Aeneas.Std.bind (discard_node_counts dl in_ids)
          (fun dc => Aeneas.Std.bind (persist_fence_node_counts dc in_ids)
          (fun pf => Aeneas.Std.bind (persist_hist_node_counts pf in_ids)
          (fun ph => ok (dl && dc && pf && ph))))) = ok v) ↔
        v = is_local := by
  intro is_local in_ids v
  have hdl : discard_leader_local is_local = ok is_local :=
    gate_of_iff is_local _ (discard_leader_local_fate_iff is_local)
  have hdc : discard_node_counts is_local in_ids = ok is_local :=
    gate_of_iff is_local _ (discard_node_counts_fate_iff is_local in_ids)
  have hpf : persist_fence_node_counts is_local in_ids = ok is_local :=
    gate_of_iff is_local _ (persist_fence_node_counts_fate_iff is_local in_ids)
  have hph : persist_hist_node_counts is_local in_ids = ok is_local :=
    gate_of_iff is_local _ (persist_hist_node_counts_fate_iff is_local in_ids)
  simp only [hdl, bind_ok, hdc, hpf, hph]
  cases is_local <;> simp
