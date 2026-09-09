-- Theorems over Aeneas extract of cf_kernel.rs (RFC-0150 P0).
-- Charon --start-from catalog entries; cf_encode_effective / decode_cf_key
-- patched (lifetime/'a str bottoms) in aeneas_cf.sh.
import Aeneas
import CfKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_cf_kernel

/-- AS-IS dente: every key is in-family. -/
theorem key_in_cf_family_as_is_dente (k f) :
    key_in_cf_family_as_is k f = ok true := by
  unfold key_in_cf_family_as_is
  rfl

/-- Catalog entry: extracted effective prefix is eq-default then empty-or-cf. -/
theorem cf_encode_effective_is_if (cf default_raw) :
    cf_encode_effective cf default_raw = (
      do
        let b ← Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default")
        if b && default_raw then ok (toStr "") else ok cf
    ) := by
  unfold cf_encode_effective
  rfl

/-- Catalog entry: no bounds ⇒ mixed/empty tag. -/
theorem infer_sst_cf_none_none :
    infer_sst_cf none none = alloc.string.String.new := by
  unfold infer_sst_cf
  rfl

/-- AS-IS dente: compact rewrites every SST. -/
theorem compact_rewrites_sst_cf_as_is_dente (s f) :
    compact_rewrites_sst_cf_as_is s f = ok true := by
  unfold compact_rewrites_sst_cf_as_is
  rfl

/-- Catalog entry: the extract axiomatizes str-eq and `is_empty`; given they
compute as rustc does on `"default"`/`""`, eq-default + default_raw ⇒ decode
is identity. -/
theorem decode_cf_key_default_raw_is_identity
    (heq : Str.Insts.CoreCmpPartialEqStr.eq (toStr "default") (toStr "default") = ok true)
    (hempty : core.str.Str.is_empty (toStr "") = ok true)
    (s : Slice Std.U8) :
    decode_cf_key (toStr "default") s true = ok s := by
  unfold decode_cf_key
  simp [cf_encode_effective, heq, hempty]

/-- Catalog entry: same axiom boundary ⇒ encode copies the bare key. -/
theorem encode_cf_key_default_raw_is_key
    (heq : Str.Insts.CoreCmpPartialEqStr.eq (toStr "default") (toStr "default") = ok true)
    (hempty : core.str.Str.is_empty (toStr "") = ok true)
    (k : Slice Std.U8) :
    encode_cf_key (toStr "default") k true =
      alloc.slice.Slice.to_vec core.clone.CloneU8 k := by
  unfold encode_cf_key
  simp [cf_encode_effective, heq, hempty]
