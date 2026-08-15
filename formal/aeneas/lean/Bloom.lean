-- Theorems over the Aeneas extract of production bloom.rs (RFC-0030).
-- Lengths are `x.val.length : Nat`. Do not `simp [loop]`.
--
-- Green: T4 on the extracted `may_contain` / `is_active` / `always_true`.
-- Remaining: loop-level T1 (`insert` then `may_contain`) and T2 roundtrip —
-- see EXTRACT.md.
import Aeneas
import BloomKernel
open Aeneas.Std Result
open Aeneas.Std.WP
open pedra_aeneas_bloom_kernel

/-- T4: `nbits = 0` ⇒ `may_contain` is `ok true` for every key. -/
theorem may_contain_nbits_zero
    (bits : alloc.vec.Vec U8) (k : U32) (key : Aeneas.Std.Slice U8) :
    BloomFilter.may_contain { bits := bits, nbits := 0#u32, k := k } key
      = ok true := by
  unfold BloomFilter.may_contain BloomFilter.is_active
  rfl

/-- T4: `k = 0` ⇒ `may_contain` is `ok true` for every key. -/
theorem may_contain_k_zero
    (bits : alloc.vec.Vec U8) (nbits : U32) (key : Aeneas.Std.Slice U8) :
    BloomFilter.may_contain { bits := bits, nbits := nbits, k := 0#u32 } key
      = ok true := by
  unfold BloomFilter.may_contain BloomFilter.is_active
  split
  · have hk : (0#u32 > 0#u32) = false := by native_decide
    simp only [hk]
    rfl
  · rfl

/-- T4: the writer-empty filter never rejects. -/
theorem always_true_never_rejects (key : Aeneas.Std.Slice U8) :
    (do
      let f ← BloomFilter.always_true
      BloomFilter.may_contain f key) = ok true := by
  unfold BloomFilter.always_true
  simp only [bind_tc_ok]
  exact may_contain_nbits_zero (alloc.vec.Vec.new U8) 0#u32 key
