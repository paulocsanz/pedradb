-- Theorems over Aeneas extract of auth_kernel.rs (Bearer scheme).
-- Charon --exclude str Pattern methods; bearer axiom and authorization
-- Iterator patched in aeneas_auth.sh.
import Aeneas
import AuthKernel
open Aeneas.Std Result
open pedra_aeneas_auth_kernel

/-- Catalog entry: ascii_lower is the uppercase test then to_ascii_lowercase. -/
theorem ascii_lower_is_extract (b) :
    ascii_lower b = (do
      let b1 ← core.num.U8.is_ascii_uppercase b
      if b1
      then core.num.U8.to_ascii_lowercase b
      else ok b) := by
  unfold ascii_lower
  rfl

/-- Catalog entry: ascii_upper is the lowercase test then to_ascii_uppercase. -/
theorem ascii_upper_is_extract (b) :
    ascii_upper b = (do
      let b1 ← core.num.U8.is_ascii_lowercase b
      if b1
      then core.num.U8.to_ascii_uppercase b
      else ok b) := by
  unfold ascii_upper
  rfl

/-- AS-IS dente: only the two literal scheme tokens. -/
theorem is_bearer_scheme_as_is_is_or (s) :
    is_bearer_scheme_as_is s = (do
      let b ← Str.Insts.CoreCmpPartialEqStr.eq s (toStr "Bearer")
      if b
      then ok true
      else Str.Insts.CoreCmpPartialEqStr.eq s (toStr "bearer")) := by
  unfold is_bearer_scheme_as_is
  rfl
