-- Theorems over Aeneas extract of path_kernel.rs (origin-form routing).
-- Charon --exclude str Pattern methods; catalog-fn holes patched in
-- aeneas_path.sh.
import Aeneas
import PathKernel
open Aeneas.Std Result
open pedra_aeneas_path_kernel

/-- Catalog entry: authority-form targets are stripped. -/
theorem strip_authority_for_routing_true :
    strip_authority_for_routing true = ok true := by
  unfold strip_authority_for_routing
  rfl

/-- AS-IS dente: never strip authority. -/
theorem strip_authority_for_routing_as_is_dente :
    strip_authority_for_routing_as_is true = ok false := by
  unfold strip_authority_for_routing_as_is
  rfl

/-- AS-IS dente: fragment stays in the path. -/
theorem strip_uri_fragment_as_is_id (t) :
    strip_uri_fragment_as_is t = ok t := by
  unfold strip_uri_fragment_as_is
  rfl

/-- AS-IS dente: Host is never compared. -/
theorem host_authority_mismatch_as_is_dente (h a) :
    host_authority_mismatch_as_is h a = ok false := by
  unfold host_authority_mismatch_as_is
  rfl

/-! ### RFC-0216 P2.1 — path ×8 átomo -/

private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- RFC-0216 P2.1 1/8 (átomo `catalog:strip_authority_for_routing`):
  o roteador da forma-authority repassa exatamente a bandeira de
  forma-authority; o AS-IS nunca strips (dente já provado acima). -/
theorem strip_authority_for_routing_fate_iff :
    ∀ (b : Bool) (r : Bool),
      (strip_authority_for_routing b = ok r) ↔ r = b := by
  intro b r
  constructor
  · intro hval
    unfold strip_authority_for_routing at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold strip_authority_for_routing
    rw [hr]
