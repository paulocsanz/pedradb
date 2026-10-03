-- Theorems over Aeneas extract of snapshot_kernel.rs
-- Payment is the linked rustc bodies; the former cfg(verus_keep_ghost)
-- stand-in was deleted. Fail-closed: this file must not contain a hole.
import Aeneas
import SnapshotKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_snapshot_kernel

/-- An unreserved user key is touched by the snapshot. -/
theorem snapshot_touches_user_key_unreserved :
    snapshot_touches_user_key false = ok true := by
  unfold snapshot_touches_user_key
  rfl

/-- A reserved key is left untouched by the snapshot. -/
theorem snapshot_touches_user_key_reserved_refuses :
    snapshot_touches_user_key true = ok false := by
  unfold snapshot_touches_user_key
  simp

/-- Snapshot must clear TX meta leftovers. -/
theorem snapshot_needs_txn_meta_clear_teeth :
    snapshot_needs_txn_meta_clear = ok true := by
  rfl

/-- AS-IS dente: leftover TX meta survives the snapshot. -/
theorem snapshot_needs_txn_meta_clear_as_is_dente :
    snapshot_needs_txn_meta_clear_as_is = ok false := by
  rfl

/-- RFC-0218 P1.3 2/11 (átomo `catalog:snap_txn_clear`, entrada
    `snapshot_needs_txn_meta_clear`): restaurar snapshot SEMPRE exige
    limpar o metadado de txn — EXATAMENTE a constante citada true.
    O AS-IS é false (txn meta vaza entre snapshots — dente
    plantado). -/
theorem snapshot_needs_txn_meta_clear_fate_iff :
    ∀ (v : Bool),
      (snapshot_needs_txn_meta_clear = ok v) ↔ (v = true) := by
  intro v
  constructor
  · intro hval
    unfold snapshot_needs_txn_meta_clear at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P1.3 6/11 (átomo `catalog:snapshot`, entrada
    `snapshot_touches_user_key`): o snapshot toca chave de usuário
    EXATAMENTE quando a chave NÃO é reservada — o lift citado
    `¬ is_reserved`. O AS-IS é a constante true (toca até reservada —
    dente plantado). -/
theorem snapshot_touches_user_key_fate_iff :
    ∀ (is_reserved : Bool) (v : Bool),
      (snapshot_touches_user_key is_reserved = ok v) ↔
      (v = decide (¬ (is_reserved = true))) := by
  intro is_reserved v
  constructor
  · intro hval
    unfold snapshot_touches_user_key at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl
