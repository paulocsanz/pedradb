-- Theorems over the Aeneas extract of production bloom.rs (RFC-0030).
-- Lengths are `x.val.length : Nat`. Do not `simp [loop]`.
--
-- Green: T4 on the extracted `may_contain` / `is_active` / `always_true`,
-- and T1 core (`set_bit` then `test_bit` on the same index).
-- Remaining: compose `insert_loop` with `may_contain_loop` (k-step).
-- T2 roundtrip — see EXTRACT.md.
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

/-! ## T1 core: set then test the same probe bit (extracted production fns). -/

private theorem u8_shift1_ne_zero (k : Nat) (hk : k < 8) :
    ((1 : Nat) <<< k) % U8.size ≠ 0 := by
  match k with
  | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 => native_decide
  | n + 8 => omega

/-- Setting probe bit `i` then testing `i` answers `true`.
    Bound: the byte index `i / 8` sits inside the bits slice. -/
theorem set_bit_test_bit_same (s : Aeneas.Std.Slice U8) (i : Usize)
    (h : i.val / 8 < s.val.length) :
    spec (do
      let s1 ← set_bit s i
      test_bit s1 i) (fun b => b = true) := by
  unfold set_bit test_bit
  have h8 : (8#usize).val ≠ 0 := by decide
  have h8n : (↑(8#usize) : Nat) ≠ 0 := by decide
  step as ⟨ i1, hi1 ⟩
  have hi1lt : i1.val < 8 := by
    rw [show i1.val = i.val % 8 from hi1]
    exact Nat.mod_lt _ (by decide)
  step as ⟨ i2, hi2 ⟩
  step as ⟨ i3, hi3 ⟩
  have hi3b : i3.val < s.length := by
    rw [show i3.val = i.val / 8 from hi3]
    simpa [Aeneas.Std.Slice.length] using h
  step as ⟨ i4, hi4 ⟩
  step as ⟨ i5, hi5 ⟩
  step as ⟨ s1, hs1 ⟩
  step as ⟨ j3, hj3 ⟩
  have hj3b : j3.val < s1.length := by
    have hlen : s1.length = s.length := by
      simp [hs1, Slice.set, Slice.setAtNat, Aeneas.Std.Slice.length]
    rw [show j3.val = i.val / 8 from hj3, hlen]
    simpa [Aeneas.Std.Slice.length] using h
  step as ⟨ x, hx ⟩
  step as ⟨ j1, hj1 ⟩
  have hj1lt : j1.val < 8 := by
    rw [show j1.val = i.val % 8 from hj1]
    exact Nat.mod_lt _ (by decide)
  step as ⟨ j4, hj4 ⟩
  step as ⟨ anded, handed ⟩
  have hi2nz : (i2 != 0#u8) = true := by
    have : i2.val ≠ 0 := by
      simpa [hi2] using u8_shift1_ne_zero i1.val hi1lt
    simpa [bne_iff_ne]
  have hj4i2 : j4 = i2 :=
    UScalar.eq_imp _ _ (by
      have hji : j1.val = i1.val := by simp [hj1, hi1]
      have : j4.val = (1 <<< i1.val) % U8.size := by simpa [hji] using hj4
      exact this.trans (Eq.symm hi2))
  have hi5eq : i5 = i4 ||| i2 :=
    UScalar.eq_imp _ _ (by simpa using hi5)
  have hbyte : x = i4 ||| i2 := by
    have hij : j3.val = i3.val := by simp [hj3, hi3]
    have : s1.val[j3.val] = i5 := by
      simp [hs1, Slice.set_val_eq, hij]
    simpa [hx, this] using hi5eq
  have hand : (i4 ||| i2) &&& i2 = i2 := by
    apply UScalar.eq_imp
    change ((i4.bv ||| i2.bv) &&& i2.bv).toNat = i2.bv.toNat
    apply congrArg BitVec.toNat
    apply BitVec.eq_of_getLsbD_eq
    intro n
    simp [BitVec.getLsbD_and, BitVec.getLsbD_or]
    tauto
  have handed' : anded = x &&& j4 :=
    UScalar.eq_imp _ _ (by simpa using handed)
  have : (anded != 0#u8) = true := by
    simp [handed', hbyte, hj4i2, hand, hi2nz]
  simpa [spec_ok] using this

/-! ## T1 loop (k = 1): insert_loop then may_contain_loop on the extract. -/

theorem probe_bit_lt (h1 h2 : U64) (i : U32) (nbits : U64)
    (hnz : nbits.val ≠ 0) :
    spec (probe_bit h1 h2 i nbits) (fun b => b.val < nbits.val) := by
  unfold probe_bit
  simp [lift]
  have hnzN : (↑nbits : Nat) ≠ 0 := hnz
  have hrem :=
    U64.rem_spec
      (core.num.U64.wrapping_add h1
        (core.num.U64.wrapping_mul (core.convert.num.FromU64U32.from i) h2))
      hnzN
  apply spec_mono hrem
  intro z hz
  have hz' : z.val = _ % nbits.val := hz
  simpa [hz'] using Nat.mod_lt _ (Nat.pos_of_ne_zero hnzN)

theorem bit_index_of_le_u32max (bit : U64)
    (hle : bit.val ≤ U32.max) :
    bit_index bit = ok (UScalar.cast .Usize (UScalar.cast .U32 bit)) := by
  unfold bit_index
  simp [lift]
  have hmxv : (core.convert.num.FromU64U32.from core.num.U32.MAX).val = (core.num.U32.MAX).val :=
    core.convert.num.FromU64U32.from_val_eq _
  have hmax : (core.num.U32.MAX).val = U32.max := by native_decide
  have : ¬ U32.rMax < bit.val := by
    have hr : U32.rMax = U32.max := by native_decide
    omega
  simp [this]
