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
