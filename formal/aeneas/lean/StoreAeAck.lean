-- Theorems over Aeneas extract of store ae_ack_kernel.rs (F48).
import Aeneas
import StoreAeAckKernel
open Aeneas.Std Result
open pedra_aeneas_store_ae_ack_kernel

/-- Dirty log without persist does not ack success. -/
theorem ae_ack_success_dirty_without_persist :
    ae_ack_success true false = ok false := by
  unfold ae_ack_success
  rfl

/-- AS-IS dente: persist failure still acks. -/
theorem ae_ack_success_as_is_dente :
    ae_ack_success_as_is true false = ok true := by
  unfold ae_ack_success_as_is
  rfl
