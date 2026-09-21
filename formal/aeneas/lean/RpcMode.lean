-- Theorems over Aeneas extract of rpc_mode_kernel.rs
-- (RFC-0067 P0 DST Queued-RPC pin). Payment is the linked rustc bodies;
-- the former cfg(verus_keep_ghost) stand-in was deleted. No holes here.
import Aeneas
import RpcModeKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_rpc_mode_kernel

/-- DST pin teeth: a pinned Queued world refuses a Direct request. -/
theorem allow_direct_rpc_pin_refuses :
    allow_direct_rpc true true = ok false := by
  unfold allow_direct_rpc
  rfl

/-- DST pin teeth: unpinned still admits Direct. -/
theorem allow_direct_rpc_unpinned_allows_direct :
    allow_direct_rpc false true = ok true := by
  unfold allow_direct_rpc
  rfl

/-- DST pin teeth: a Queued request is always admitted. -/
theorem allow_direct_rpc_queued_always_admitted :
    allow_direct_rpc true false = ok true := by
  unfold allow_direct_rpc
  rfl

/-- AS-IS 0067 tooth: the pin does not stick — Direct always admitted. -/
theorem allow_direct_rpc_as_is_tooth :
    allow_direct_rpc_as_is true true = ok true := by
  unfold allow_direct_rpc_as_is
  rfl

/-- RFC-0218 P1.3 4/11 (atom `catalog:rpc_mode`, entrada
    `allow_direct_rpc`): o RPC direto é EXATAMENTE o despacho citado
    — sem pedido direto, sempre true; com pedido direto, só se o pin
    de destino não for liderado. O AS-IS não olha dst_pin (RPC direto
    contra o líder — tooth plantado). -/
theorem allow_direct_rpc_fate_iff :
    ∀ (dst_pin : Bool) (want_direct : Bool) (v : Bool),
      (allow_direct_rpc dst_pin want_direct = ok v) ↔
      ((want_direct = true ∧ dst_pin = true ∧ v = false) ∨
       (want_direct = true ∧ dst_pin = false ∧ v = true) ∨
       (want_direct = false ∧ v = true)) := by
  intro dst_pin want_direct v
  constructor
  · intro hval
    unfold allow_direct_rpc at hval
    split at hval
    · next hw =>
      split at hval
      · next hd => injection hval with hv; exact Or.inl ⟨hw, hd, hv.symm⟩
      · next hd =>
        simp only [Bool.not_eq_true] at hd
        injection hval with hv
        exact Or.inr (Or.inl ⟨hw, hd, hv.symm⟩)
    · next hw =>
      simp only [Bool.not_eq_true] at hw
      injection hval with hv
      exact Or.inr (Or.inr ⟨hw, hv.symm⟩)
  · rintro (⟨hw, hd, hv⟩ | ⟨hw, hd, hv⟩ | ⟨hw, hv⟩)
    · subst hw; subst hd; subst hv; rfl
    · subst hw; subst hd; subst hv; rfl
    · subst hw; subst hv; rfl
