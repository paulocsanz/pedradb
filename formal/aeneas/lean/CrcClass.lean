-- CRC32C detection class (RFC-0228). Pure-math theorems over GF(2)[X]:
-- generator G = 0x11EDC6F41 (Castagnoli) equals (X+1)*P31 with
-- P31 = 0xF5B4253F. Undetected corruption E <-> G divides E. Detected
-- classes proven here: burst <= 32 bits, odd Hamming weight, 2-bit
-- errors with span < 2^31-1 (order of X in GF(2)[X]/(P31) is N =
-- 2^31-1, certified by a Frobenius chain of 30 certificates), weight
-- <= 3, and codeword+error closure. The channel question (does NAND
-- corruption stay inside this class) remains R-hardware/R-crc.
-- No Aeneas dependency: pure Mathlib.
import Mathlib

open Polynomial

set_option maxHeartbeats 4000000
set_option maxRecDepth 300000

noncomputable section

/-! ### polyOf: um polinômio como soma de monômios sobre um Finset -/

/-- polyOf s = ∑ i ∈ s, X ^ i sobre GF(2). -/
def polyOf (s : Finset ℕ) : Polynomial (ZMod 2) := ∑ i ∈ s, X ^ i

theorem polyOf_empty : polyOf (∅ : Finset ℕ) = 0 := by simp [polyOf]

theorem polyOf_singleton (a : ℕ) : polyOf ({a} : Finset ℕ) = X ^ a := by
  unfold polyOf; simp

theorem polyOf_pair (a b : ℕ) (h : a ≠ b) : polyOf ({a, b} : Finset ℕ) = X ^ a + X ^ b := by
  unfold polyOf
  rw [Finset.sum_insert (by simp [h]), Finset.sum_singleton]

theorem polyOf_zero_set : polyOf ({0} : Finset ℕ) = 1 := by
  rw [polyOf_singleton, pow_zero]

/-- X + 1 como polyOf. -/
theorem X_add_one_eq : (X + 1 : Polynomial (ZMod 2)) = polyOf {0, 1} := by
  rw [polyOf_pair 0 1 (by decide), pow_zero, pow_one, add_comm]

theorem polyOf_mul (s t : Finset ℕ) :
    polyOf s * polyOf t = ∑ p ∈ s ×ˢ t, X ^ (p.1 + p.2) := by
  unfold polyOf
  rw [Finset.sum_mul_sum, Finset.sum_product]
  exact Finset.sum_congr rfl fun i _ => Finset.sum_congr rfl fun j _ => by rw [pow_add]

/-! ### Característica 2 -/

theorem two_eq_zero_poly : (2 : Polynomial (ZMod 2)) = 0 :=
  CharP.cast_eq_zero (R := Polynomial (ZMod 2)) 2

theorem self_add_zero (a : Polynomial (ZMod 2)) : a + a = 0 := by
  rw [← two_mul, two_eq_zero_poly, zero_mul]

theorem sq_add (a b : Polynomial (ZMod 2)) : (a + b) ^ 2 = a ^ 2 + b ^ 2 := by
  have h2 : (2 : Polynomial (ZMod 2)) * a * b = 0 := by
    rw [mul_assoc, two_eq_zero_poly, zero_mul]
  rw [add_sq, h2, add_zero]

theorem X_pow_sq (e : ℕ) : (X ^ e : Polynomial (ZMod 2)) ^ 2 = X ^ (2 * e) := by
  rw [← pow_mul, Nat.mul_comm]

theorem pair_cancel (a c b : Polynomial (ZMod 2)) : a + c + (c + b) = a + b := by
  rw [add_assoc a c (c + b), ← add_assoc c c b, self_add_zero, zero_add]

/-! ### Quadrado de polyOf e grau das somas de monômios -/

theorem polyOf_insert (a : ℕ) (s : Finset ℕ) (h : a ∉ s) :
    polyOf (insert a s) = X ^ a + polyOf s := by
  unfold polyOf
  rw [Finset.sum_insert h]

theorem polyOf_sq (s : Finset ℕ) :
    (polyOf s) ^ 2 = ∑ e ∈ s, X ^ (2 * e) := by
  induction s using Finset.induction with
  | empty => simp [polyOf]
  | insert a s ha ih =>
    rw [polyOf_insert a s ha, Finset.sum_insert ha, sq_add, ih]
    congr 1
    exact X_pow_sq a

theorem sum_mul_X (s : Finset ℕ) :
    ((∑ e ∈ s, X ^ (2 * e) : Polynomial (ZMod 2))) * X = ∑ e ∈ s, X ^ (2 * e + 1) := by
  rw [Finset.sum_mul]
  exact Finset.sum_congr rfl fun e _ => by rw [pow_succ]

/-- Coeficiente de soma de monômios com expoentes < n é 0 a partir de n. -/
theorem coeff_sum_X_pow_zero {α : Type*} (S : Finset α) (g : α → ℕ) {n j : ℕ}
    (hj : n ≤ j) (hS : ∀ x ∈ S, g x < n) :
    (∑ x ∈ S, (X ^ g x : Polynomial (ZMod 2))).coeff j = 0 := by
  classical
  rw [Polynomial.finsetSum_coeff]
  refine Finset.sum_eq_zero fun x hx => ?_
  rw [Polynomial.coeff_X_pow]
  by_cases hg : j = g x
  · exfalso
    have h1 := hS x hx
    rw [← hg] at h1
    exact absurd h1 (Nat.not_lt.mpr hj)
  · exact if_neg hg

/-! ### Literais: P31 = 0xF5B4253F, GG = 0x11EDC6F41 -/

/-- P31 = 0xF5B4253F (grau 31, termo constante 1). -/
def P31 : Polynomial (ZMod 2) := polyOf {0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31}

/-- GG = 0x11EDC6F41 = (X+1)·P31 (grau 32, termo constante 1). -/
def GG : Polynomial (ZMod 2) := polyOf {0, 6, 8, 9, 10, 11, 13, 14, 18, 19, 20, 22, 23, 25, 26, 27, 28, 32}

/-- Fatoração (X+1)·P31 = G, certificada coeficiente a coeficiente. -/
theorem GG_factor : polyOf {0, 1} * polyOf {0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} = GG := by
  unfold GG
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.finsetSum_coeff, Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero (({0, 1} : Finset ℕ) ×ˢ ({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 6, 8, 9, 10, 11, 13, 14, 18, 19, 20, 22, 23, 25, 26, 27, 28, 32} : Finset ℕ) (fun i => i) h64 (by decide)]

theorem GG_eq : (X + 1 : Polynomial (ZMod 2)) * P31 = GG := by
  rw [X_add_one_eq]
  unfold P31
  exact GG_factor

theorem P31_coeff_31 : P31.coeff 31 = 1 := by
  simp only [P31, polyOf, Polynomial.finsetSum_coeff, Polynomial.coeff_X_pow]
  decide

theorem P31_coeff_0 : P31.coeff 0 = 1 := by
  simp only [P31, polyOf, Polynomial.finsetSum_coeff, Polynomial.coeff_X_pow]
  decide

theorem GG_coeff_32 : GG.coeff 32 = 1 := by
  simp only [GG, polyOf, Polynomial.finsetSum_coeff, Polynomial.coeff_X_pow]
  decide

theorem GG_coeff_0 : GG.coeff 0 = 1 := by
  simp only [GG, polyOf, Polynomial.finsetSum_coeff, Polynomial.coeff_X_pow]
  decide

theorem P31_coeff_zero_ge (j : ℕ) (hj : 32 ≤ j) : P31.coeff j = 0 := by
  simp only [P31, polyOf]
  exact coeff_sum_X_pow_zero ({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) (fun i => i) hj (by decide)

theorem GG_coeff_zero_ge (j : ℕ) (hj : 33 ≤ j) : GG.coeff j = 0 := by
  simp only [GG, polyOf]
  exact coeff_sum_X_pow_zero ({0, 6, 8, 9, 10, 11, 13, 14, 18, 19, 20, 22, 23, 25, 26, 27, 28, 32} : Finset ℕ) (fun i => i) hj (by decide)

theorem P31_ne_zero : P31 ≠ 0 := by
  intro h
  have hc := P31_coeff_31
  rw [h, Polynomial.coeff_zero] at hc
  exact zero_ne_one hc

theorem GG_ne_zero : GG ≠ 0 := by
  intro h
  have hc := GG_coeff_32
  rw [h, Polynomial.coeff_zero] at hc
  exact zero_ne_one hc

theorem P31_natDegree : P31.natDegree = 31 := by
  refine le_antisymm ?_ ((le_natDegree_of_ne_zero (by rw [P31_coeff_31]; exact one_ne_zero)))
  by_contra h
  have h32 : 32 ≤ P31.natDegree := by omega
  have hlc : P31.coeff P31.natDegree ≠ 0 :=
    Polynomial.leadingCoeff_ne_zero.mpr P31_ne_zero
  rw [P31_coeff_zero_ge _ h32] at hlc
  exact hlc rfl

theorem GG_natDegree : GG.natDegree = 32 := by
  refine le_antisymm ?_ ((le_natDegree_of_ne_zero (by rw [GG_coeff_32]; exact one_ne_zero)))
  by_contra h
  have h33 : 33 ≤ GG.natDegree := by omega
  have hlc : GG.coeff GG.natDegree ≠ 0 :=
    Polynomial.leadingCoeff_ne_zero.mpr GG_ne_zero
  rw [GG_coeff_zero_ge _ h33] at hlc
  exact hlc rfl

/-! ### Cola simbólica da cadeia de Frobenius -/

/-- Um certificado + hipótese indutiva avança um passo: se
X^e + c_k = P31·t e c_k²·X + c_{k+1} = P31·Q então X^{2e+1} + c_{k+1} = P31·t'. -/
theorem chain_step (A B Q : Finset ℕ) {e : ℕ} (t : Polynomial (ZMod 2))
    (cert : polyOf A ^ 2 * X + polyOf B = P31 * polyOf Q)
    (ih : X ^ e + polyOf A = P31 * t) :
    ∃ t', X ^ (2 * e + 1) + polyOf B = P31 * t' := by
  have hsq : X ^ (2 * e) + polyOf A ^ 2 = P31 * (P31 * t ^ 2) := by
    have h := congrArg (fun z => z ^ 2) ih
    rw [sq_add, X_pow_sq, mul_pow, pow_two P31, mul_assoc] at h
    exact h
  have hX : X ^ (2 * e + 1) + polyOf A ^ 2 * X = P31 * (P31 * t ^ 2) * X := by
    have h2 := congrArg (fun z => z * X) hsq
    rw [add_mul, ← pow_succ] at h2
    exact h2
  refine ⟨P31 * t ^ 2 * X + polyOf Q, ?_⟩
  have key : X ^ (2 * e + 1) + polyOf B
      = (X ^ (2 * e + 1) + polyOf A ^ 2 * X) + (polyOf A ^ 2 * X + polyOf B) := by
    rw [pair_cancel]
  rw [key, hX, cert]
  ring
/-! ### Cadeia de Frobenius: 30 certificados computacionais -/

-- (dados gerados e conferidos offline; produto máximo 399 pares por coeficiente)

theorem cert1 :
    polyOf {1} ^ 2 * X + polyOf {3} = P31 * polyOf {} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({1} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({3} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert2 :
    polyOf {3} ^ 2 * X + polyOf {7} = P31 * polyOf {} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({3} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({7} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert3 :
    polyOf {7} ^ 2 * X + polyOf {15} = P31 * polyOf {} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({7} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({15} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert4 :
    polyOf {15} ^ 2 * X + polyOf {0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30} = P31 * polyOf {0} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({15} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert5 :
    polyOf {0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30} ^ 2 * X + polyOf {0, 1, 3, 4, 5, 7, 9, 13, 15, 16, 18, 20, 22, 24, 26, 27, 28} = P31 * polyOf {0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 16, 20, 21, 24, 25, 27, 28, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 1, 3, 4, 5, 7, 9, 13, 15, 16, 18, 20, 22, 24, 26, 27, 28} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 16, 20, 21, 24, 25, 27, 28, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert6 :
    polyOf {0, 1, 3, 4, 5, 7, 9, 13, 15, 16, 18, 20, 22, 24, 26, 27, 28} ^ 2 * X + polyOf {2, 3, 7, 9, 10, 11, 15, 16, 17, 19, 21, 26, 27} = P31 * polyOf {1, 3, 7, 11, 16, 17, 20, 21, 23, 24, 25, 26} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 1, 3, 4, 5, 7, 9, 13, 15, 16, 18, 20, 22, 24, 26, 27, 28} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({2, 3, 7, 9, 10, 11, 15, 16, 17, 19, 21, 26, 27} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({1, 3, 7, 11, 16, 17, 20, 21, 23, 24, 25, 26} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert7 :
    polyOf {2, 3, 7, 9, 10, 11, 15, 16, 17, 19, 21, 26, 27} ^ 2 * X + polyOf {0, 2, 6, 11, 13, 15, 16, 20, 22, 23, 26, 27, 29} = P31 * polyOf {0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 12, 14, 18, 20, 21, 22, 23, 24} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({2, 3, 7, 9, 10, 11, 15, 16, 17, 19, 21, 26, 27} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 2, 6, 11, 13, 15, 16, 20, 22, 23, 26, 27, 29} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 12, 14, 18, 20, 21, 22, 23, 24} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert8 :
    polyOf {0, 2, 6, 11, 13, 15, 16, 20, 22, 23, 26, 27, 29} ^ 2 * X + polyOf {0, 4, 7, 11, 12, 13, 14, 15, 16, 20, 23, 24, 25, 26, 27, 29} = P31 * polyOf {0, 2, 4, 7, 8, 9, 10, 11, 13, 20, 21, 22, 23, 27, 28} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 2, 6, 11, 13, 15, 16, 20, 22, 23, 26, 27, 29} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 4, 7, 11, 12, 13, 14, 15, 16, 20, 23, 24, 25, 26, 27, 29} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 2, 4, 7, 8, 9, 10, 11, 13, 20, 21, 22, 23, 27, 28} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert9 :
    polyOf {0, 4, 7, 11, 12, 13, 14, 15, 16, 20, 23, 24, 25, 26, 27, 29} ^ 2 * X + polyOf {0, 4, 8, 10, 11, 14, 15, 18, 19, 20, 24, 27} = P31 * polyOf {0, 2, 4, 5, 6, 8, 9, 10, 11, 16, 17, 18, 19, 21, 22, 23, 27, 28} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 4, 7, 11, 12, 13, 14, 15, 16, 20, 23, 24, 25, 26, 27, 29} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 4, 8, 10, 11, 14, 15, 18, 19, 20, 24, 27} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 2, 4, 5, 6, 8, 9, 10, 11, 16, 17, 18, 19, 21, 22, 23, 27, 28} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert10 :
    polyOf {0, 4, 8, 10, 11, 14, 15, 18, 19, 20, 24, 27} ^ 2 * X + polyOf {0, 1, 2, 4, 8, 14, 16, 21, 23, 24, 26, 27, 28, 30} = P31 * polyOf {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 17, 18, 20, 23, 24} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 4, 8, 10, 11, 14, 15, 18, 19, 20, 24, 27} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 1, 2, 4, 8, 14, 16, 21, 23, 24, 26, 27, 28, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 17, 18, 20, 23, 24} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert11 :
    polyOf {0, 1, 2, 4, 8, 14, 16, 21, 23, 24, 26, 27, 28, 30} ^ 2 * X + polyOf {0, 1, 2, 4, 7, 8, 13, 14, 16, 17, 23, 25, 28} = P31 * polyOf {0, 1, 2, 11, 13, 15, 18, 21, 23, 24, 25, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 1, 2, 4, 8, 14, 16, 21, 23, 24, 26, 27, 28, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 1, 2, 4, 7, 8, 13, 14, 16, 17, 23, 25, 28} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 2, 11, 13, 15, 18, 21, 23, 24, 25, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert12 :
    polyOf {0, 1, 2, 4, 7, 8, 13, 14, 16, 17, 23, 25, 28} ^ 2 * X + polyOf {2, 3, 7, 9, 10, 13, 14, 20, 21, 23, 25, 26, 27, 30} = P31 * polyOf {1, 3, 5, 6, 8, 12, 13, 14, 15, 16, 19, 20, 22, 25, 26} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 1, 2, 4, 7, 8, 13, 14, 16, 17, 23, 25, 28} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({2, 3, 7, 9, 10, 13, 14, 20, 21, 23, 25, 26, 27, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({1, 3, 5, 6, 8, 12, 13, 14, 15, 16, 19, 20, 22, 25, 26} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert13 :
    polyOf {2, 3, 7, 9, 10, 13, 14, 20, 21, 23, 25, 26, 27, 30} ^ 2 * X + polyOf {3, 4, 6, 8, 9, 12, 14, 16, 17, 18, 20, 21, 25, 27, 29} = P31 * polyOf {3, 9, 10, 11, 15, 17, 19, 20, 21, 22, 23, 24, 26, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({2, 3, 7, 9, 10, 13, 14, 20, 21, 23, 25, 26, 27, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({3, 4, 6, 8, 9, 12, 14, 16, 17, 18, 20, 21, 25, 27, 29} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({3, 9, 10, 11, 15, 17, 19, 20, 21, 22, 23, 24, 26, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert14 :
    polyOf {3, 4, 6, 8, 9, 12, 14, 16, 17, 18, 20, 21, 25, 27, 29} ^ 2 * X + polyOf {1, 2, 4, 6, 7, 8, 10, 12, 13, 14, 17, 18, 19, 21, 24, 25, 29} = P31 * polyOf {1, 3, 4, 5, 6, 8, 12, 14, 15, 18, 19, 23, 27, 28} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({3, 4, 6, 8, 9, 12, 14, 16, 17, 18, 20, 21, 25, 27, 29} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({1, 2, 4, 6, 7, 8, 10, 12, 13, 14, 17, 18, 19, 21, 24, 25, 29} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({1, 3, 4, 5, 6, 8, 12, 14, 15, 18, 19, 23, 27, 28} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert15 :
    polyOf {1, 2, 4, 6, 7, 8, 10, 12, 13, 14, 17, 18, 19, 21, 24, 25, 29} ^ 2 * X + polyOf {1, 3, 8, 12, 13, 18, 20, 22, 24, 25, 28} = P31 * polyOf {1, 2, 5, 6, 7, 9, 10, 11, 14, 15, 17, 19, 20, 24, 27, 28} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({1, 2, 4, 6, 7, 8, 10, 12, 13, 14, 17, 18, 19, 21, 24, 25, 29} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({1, 3, 8, 12, 13, 18, 20, 22, 24, 25, 28} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({1, 2, 5, 6, 7, 9, 10, 11, 14, 15, 17, 19, 20, 24, 27, 28} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert16 :
    polyOf {1, 3, 8, 12, 13, 18, 20, 22, 24, 25, 28} ^ 2 * X + polyOf {0, 5, 6, 7, 8, 9, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 28, 29} = P31 * polyOf {0, 1, 3, 4, 5, 6, 9, 14, 17, 18, 19, 20, 22, 25, 26} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({1, 3, 8, 12, 13, 18, 20, 22, 24, 25, 28} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 5, 6, 7, 8, 9, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 28, 29} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 3, 4, 5, 6, 9, 14, 17, 18, 19, 20, 22, 25, 26} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert17 :
    polyOf {0, 5, 6, 7, 8, 9, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 28, 29} ^ 2 * X + polyOf {2, 3, 4, 5, 8, 11, 12, 13, 18, 19, 20, 21, 22, 24, 25, 26, 29, 30} = P31 * polyOf {1, 6, 7, 8, 10, 11, 12, 14, 16, 17, 22, 24, 25, 26, 27, 28} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 5, 6, 7, 8, 9, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 28, 29} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({2, 3, 4, 5, 8, 11, 12, 13, 18, 19, 20, 21, 22, 24, 25, 26, 29, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({1, 6, 7, 8, 10, 11, 12, 14, 16, 17, 22, 24, 25, 26, 27, 28} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert18 :
    polyOf {2, 3, 4, 5, 8, 11, 12, 13, 18, 19, 20, 21, 22, 24, 25, 26, 29, 30} ^ 2 * X + polyOf {5, 7, 8, 10, 13, 14, 16, 17, 19, 20, 22, 24, 27, 30} = P31 * polyOf {8, 12, 13, 14, 15, 19, 21, 22, 24, 26, 27, 28, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({2, 3, 4, 5, 8, 11, 12, 13, 18, 19, 20, 21, 22, 24, 25, 26, 29, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({5, 7, 8, 10, 13, 14, 16, 17, 19, 20, 22, 24, 27, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({8, 12, 13, 14, 15, 19, 21, 22, 24, 26, 27, 28, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert19 :
    polyOf {5, 7, 8, 10, 13, 14, 16, 17, 19, 20, 22, 24, 27, 30} ^ 2 * X + polyOf {4, 6, 7, 8, 9, 13, 16, 19, 20, 23, 25, 28} = P31 * polyOf {4, 5, 6, 12, 13, 23, 24, 26, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({5, 7, 8, 10, 13, 14, 16, 17, 19, 20, 22, 24, 27, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({4, 6, 7, 8, 9, 13, 16, 19, 20, 23, 25, 28} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({4, 5, 6, 12, 13, 23, 24, 26, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert20 :
    polyOf {4, 6, 7, 8, 9, 13, 16, 19, 20, 23, 25, 28} ^ 2 * X + polyOf {1, 2, 3, 4, 9, 16, 19, 20, 21, 22, 23, 27, 28, 29, 30} = P31 * polyOf {1, 5, 7, 9, 10, 12, 13, 14, 15, 16, 19, 20, 22, 25, 26} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({4, 6, 7, 8, 9, 13, 16, 19, 20, 23, 25, 28} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({1, 2, 3, 4, 9, 16, 19, 20, 21, 22, 23, 27, 28, 29, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({1, 5, 7, 9, 10, 12, 13, 14, 15, 16, 19, 20, 22, 25, 26} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert21 :
    polyOf {1, 2, 3, 4, 9, 16, 19, 20, 21, 22, 23, 27, 28, 29, 30} ^ 2 * X + polyOf {0, 2, 5, 6, 7, 12, 14, 15, 20, 21, 23, 24, 25, 29, 30} = P31 * polyOf {0, 1, 2, 4, 9, 10, 11, 16, 17, 22, 23, 25, 27, 28, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({1, 2, 3, 4, 9, 16, 19, 20, 21, 22, 23, 27, 28, 29, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 2, 5, 6, 7, 12, 14, 15, 20, 21, 23, 24, 25, 29, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 2, 4, 9, 10, 11, 16, 17, 22, 23, 25, 27, 28, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert22 :
    polyOf {0, 2, 5, 6, 7, 12, 14, 15, 20, 21, 23, 24, 25, 29, 30} ^ 2 * X + polyOf {0, 2, 3, 6, 8, 9, 10, 13, 16, 17, 19, 22, 26, 28, 30} = P31 * polyOf {0, 4, 5, 6, 7, 9, 12, 16, 18, 19, 24, 26, 27, 28, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 2, 5, 6, 7, 12, 14, 15, 20, 21, 23, 24, 25, 29, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 2, 3, 6, 8, 9, 10, 13, 16, 17, 19, 22, 26, 28, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 4, 5, 6, 7, 9, 12, 16, 18, 19, 24, 26, 27, 28, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert23 :
    polyOf {0, 2, 3, 6, 8, 9, 10, 13, 16, 17, 19, 22, 26, 28, 30} ^ 2 * X + polyOf {0, 1, 2, 5, 6, 8, 11, 12, 14, 16, 18, 21, 22, 27, 30} = P31 * polyOf {0, 1, 2, 3, 7, 10, 11, 12, 14, 16, 17, 20, 21, 25, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 2, 3, 6, 8, 9, 10, 13, 16, 17, 19, 22, 26, 28, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 1, 2, 5, 6, 8, 11, 12, 14, 16, 18, 21, 22, 27, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 2, 3, 7, 10, 11, 12, 14, 16, 17, 20, 21, 25, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert24 :
    polyOf {0, 1, 2, 5, 6, 8, 11, 12, 14, 16, 18, 21, 22, 27, 30} ^ 2 * X + polyOf {0, 7, 9, 10, 11, 12, 13, 15, 16, 17, 19, 21, 23, 24, 27, 28} = P31 * polyOf {0, 2, 3, 4, 5, 7, 8, 9, 10, 11, 13, 14, 17, 18, 23, 24, 26, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 1, 2, 5, 6, 8, 11, 12, 14, 16, 18, 21, 22, 27, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 7, 9, 10, 11, 12, 13, 15, 16, 17, 19, 21, 23, 24, 27, 28} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 2, 3, 4, 5, 7, 8, 9, 10, 11, 13, 14, 17, 18, 23, 24, 26, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert25 :
    polyOf {0, 7, 9, 10, 11, 12, 13, 15, 16, 17, 19, 21, 23, 24, 27, 28} ^ 2 * X + polyOf {0, 1, 2, 3, 4, 6, 12, 13, 14, 17, 21, 23, 24, 27, 28} = P31 * polyOf {0, 1, 2, 5, 10, 12, 13, 14, 15, 17, 18, 20, 22, 23, 24, 25, 26} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 7, 9, 10, 11, 12, 13, 15, 16, 17, 19, 21, 23, 24, 27, 28} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 1, 2, 3, 4, 6, 12, 13, 14, 17, 21, 23, 24, 27, 28} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 2, 5, 10, 12, 13, 14, 15, 17, 18, 20, 22, 23, 24, 25, 26} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert26 :
    polyOf {0, 1, 2, 3, 4, 6, 12, 13, 14, 17, 21, 23, 24, 27, 28} ^ 2 * X + polyOf {1, 3, 4, 5, 9, 11, 12, 14, 15, 16, 17, 20, 21, 22, 26, 30} = P31 * polyOf {4, 5, 7, 8, 10, 12, 13, 14, 15, 17, 18, 20, 22, 23, 24, 25, 26} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 1, 2, 3, 4, 6, 12, 13, 14, 17, 21, 23, 24, 27, 28} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({1, 3, 4, 5, 9, 11, 12, 14, 15, 16, 17, 20, 21, 22, 26, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({4, 5, 7, 8, 10, 12, 13, 14, 15, 17, 18, 20, 22, 23, 24, 25, 26} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert27 :
    polyOf {1, 3, 4, 5, 9, 11, 12, 14, 15, 16, 17, 20, 21, 22, 26, 30} ^ 2 * X + polyOf {2, 4, 6, 7, 8, 9, 11, 15, 20, 26, 29, 30} = P31 * polyOf {2, 5, 6, 7, 9, 10, 13, 17, 20, 21, 22, 26, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({1, 3, 4, 5, 9, 11, 12, 14, 15, 16, 17, 20, 21, 22, 26, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({2, 4, 6, 7, 8, 9, 11, 15, 20, 26, 29, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({2, 5, 6, 7, 9, 10, 13, 17, 20, 21, 22, 26, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert28 :
    polyOf {2, 4, 6, 7, 8, 9, 11, 15, 20, 26, 29, 30} ^ 2 * X + polyOf {4, 6, 7, 9, 10, 12, 13, 14, 16, 19, 20, 21, 22, 25, 26, 27, 28, 29, 30} = P31 * polyOf {4, 8, 11, 14, 15, 16, 17, 18, 20, 21, 22, 24, 26, 27, 28, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({2, 4, 6, 7, 8, 9, 11, 15, 20, 26, 29, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({4, 6, 7, 9, 10, 12, 13, 14, 16, 19, 20, 21, 22, 25, 26, 27, 28, 29, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({4, 8, 11, 14, 15, 16, 17, 18, 20, 21, 22, 24, 26, 27, 28, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert29 :
    polyOf {4, 6, 7, 9, 10, 12, 13, 14, 16, 19, 20, 21, 22, 25, 26, 27, 28, 29, 30} ^ 2 * X + polyOf {0, 2, 3, 6, 7, 9, 10, 11, 13, 17, 18, 21, 23, 24, 25, 27, 28, 30} = P31 * polyOf {0, 1, 2, 4, 7, 8, 10, 11, 15, 16, 17, 18, 19, 20, 21, 23, 25, 27, 28, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({4, 6, 7, 9, 10, 12, 13, 14, 16, 19, 20, 21, 22, 25, 26, 27, 28, 29, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0, 2, 3, 6, 7, 9, 10, 11, 13, 17, 18, 21, 23, 24, 25, 27, 28, 30} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 1, 2, 4, 7, 8, 10, 11, 15, 16, 17, 18, 19, 20, 21, 23, 25, 27, 28, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

theorem cert30 :
    polyOf {0, 2, 3, 6, 7, 9, 10, 11, 13, 17, 18, 21, 23, 24, 25, 27, 28, 30} ^ 2 * X + polyOf {0} = P31 * polyOf {0, 2, 5, 7, 8, 9, 11, 12, 13, 15, 16, 19, 20, 22, 23, 24, 25, 29, 30} := by
  rw [polyOf_sq, sum_mul_X]
  unfold P31
  rw [polyOf_mul]
  ext j
  by_cases h : j < 64
  · interval_cases j
    all_goals simp only [polyOf, Polynomial.coeff_add, Polynomial.finsetSum_coeff,
      Polynomial.coeff_X_pow]
    all_goals decide
  · have h64 : 64 ≤ j := Nat.not_lt.mp h
    rw [Polynomial.coeff_add]
    simp only [polyOf]
    rw [coeff_sum_X_pow_zero ({0, 2, 3, 6, 7, 9, 10, 11, 13, 17, 18, 21, 23, 24, 25, 27, 28, 30} : Finset ℕ) (fun e => 2 * e + 1) h64 (by decide),
      coeff_sum_X_pow_zero ({0} : Finset ℕ) (fun i => i) h64 (by decide),
      coeff_sum_X_pow_zero (({0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30, 31} : Finset ℕ) ×ˢ ({0, 2, 5, 7, 8, 9, 11, 12, 13, 15, 16, 19, 20, 22, 23, 24, 25, 29, 30} : Finset ℕ)) (fun p => p.1 + p.2) h64 (by decide),
      add_zero]

-- Base: X + X = 0 = P31·0; cada passo dobra+1 o expoente até 2^31-1.
theorem chain_full : ∃ t, X ^ 2147483647 + polyOf {0} = P31 * t := by
  obtain ⟨t1, h1⟩ : ∃ t, X ^ 1 + polyOf {1} = P31 * t :=
    ⟨0, by rw [polyOf_singleton, pow_one, self_add_zero, mul_zero]⟩
  obtain ⟨t2, h2⟩ := chain_step {1} {3} {} t1 cert1 h1
  obtain ⟨t3, h3⟩ := chain_step {3} {7} {} t2 cert2 h2
  obtain ⟨t4, h4⟩ := chain_step {7} {15} {} t3 cert3 h3
  obtain ⟨t5, h5⟩ := chain_step {15} {0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30} {0} t4 cert4 h4
  obtain ⟨t6, h6⟩ := chain_step {0, 1, 2, 3, 4, 5, 8, 10, 13, 18, 20, 21, 23, 24, 26, 28, 29, 30} {0, 1, 3, 4, 5, 7, 9, 13, 15, 16, 18, 20, 22, 24, 26, 27, 28} {0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 16, 20, 21, 24, 25, 27, 28, 29, 30} t5 cert5 h5
  obtain ⟨t7, h7⟩ := chain_step {0, 1, 3, 4, 5, 7, 9, 13, 15, 16, 18, 20, 22, 24, 26, 27, 28} {2, 3, 7, 9, 10, 11, 15, 16, 17, 19, 21, 26, 27} {1, 3, 7, 11, 16, 17, 20, 21, 23, 24, 25, 26} t6 cert6 h6
  obtain ⟨t8, h8⟩ := chain_step {2, 3, 7, 9, 10, 11, 15, 16, 17, 19, 21, 26, 27} {0, 2, 6, 11, 13, 15, 16, 20, 22, 23, 26, 27, 29} {0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 12, 14, 18, 20, 21, 22, 23, 24} t7 cert7 h7
  obtain ⟨t9, h9⟩ := chain_step {0, 2, 6, 11, 13, 15, 16, 20, 22, 23, 26, 27, 29} {0, 4, 7, 11, 12, 13, 14, 15, 16, 20, 23, 24, 25, 26, 27, 29} {0, 2, 4, 7, 8, 9, 10, 11, 13, 20, 21, 22, 23, 27, 28} t8 cert8 h8
  obtain ⟨t10, h10⟩ := chain_step {0, 4, 7, 11, 12, 13, 14, 15, 16, 20, 23, 24, 25, 26, 27, 29} {0, 4, 8, 10, 11, 14, 15, 18, 19, 20, 24, 27} {0, 2, 4, 5, 6, 8, 9, 10, 11, 16, 17, 18, 19, 21, 22, 23, 27, 28} t9 cert9 h9
  obtain ⟨t11, h11⟩ := chain_step {0, 4, 8, 10, 11, 14, 15, 18, 19, 20, 24, 27} {0, 1, 2, 4, 8, 14, 16, 21, 23, 24, 26, 27, 28, 30} {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 17, 18, 20, 23, 24} t10 cert10 h10
  obtain ⟨t12, h12⟩ := chain_step {0, 1, 2, 4, 8, 14, 16, 21, 23, 24, 26, 27, 28, 30} {0, 1, 2, 4, 7, 8, 13, 14, 16, 17, 23, 25, 28} {0, 1, 2, 11, 13, 15, 18, 21, 23, 24, 25, 29, 30} t11 cert11 h11
  obtain ⟨t13, h13⟩ := chain_step {0, 1, 2, 4, 7, 8, 13, 14, 16, 17, 23, 25, 28} {2, 3, 7, 9, 10, 13, 14, 20, 21, 23, 25, 26, 27, 30} {1, 3, 5, 6, 8, 12, 13, 14, 15, 16, 19, 20, 22, 25, 26} t12 cert12 h12
  obtain ⟨t14, h14⟩ := chain_step {2, 3, 7, 9, 10, 13, 14, 20, 21, 23, 25, 26, 27, 30} {3, 4, 6, 8, 9, 12, 14, 16, 17, 18, 20, 21, 25, 27, 29} {3, 9, 10, 11, 15, 17, 19, 20, 21, 22, 23, 24, 26, 29, 30} t13 cert13 h13
  obtain ⟨t15, h15⟩ := chain_step {3, 4, 6, 8, 9, 12, 14, 16, 17, 18, 20, 21, 25, 27, 29} {1, 2, 4, 6, 7, 8, 10, 12, 13, 14, 17, 18, 19, 21, 24, 25, 29} {1, 3, 4, 5, 6, 8, 12, 14, 15, 18, 19, 23, 27, 28} t14 cert14 h14
  obtain ⟨t16, h16⟩ := chain_step {1, 2, 4, 6, 7, 8, 10, 12, 13, 14, 17, 18, 19, 21, 24, 25, 29} {1, 3, 8, 12, 13, 18, 20, 22, 24, 25, 28} {1, 2, 5, 6, 7, 9, 10, 11, 14, 15, 17, 19, 20, 24, 27, 28} t15 cert15 h15
  obtain ⟨t17, h17⟩ := chain_step {1, 3, 8, 12, 13, 18, 20, 22, 24, 25, 28} {0, 5, 6, 7, 8, 9, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 28, 29} {0, 1, 3, 4, 5, 6, 9, 14, 17, 18, 19, 20, 22, 25, 26} t16 cert16 h16
  obtain ⟨t18, h18⟩ := chain_step {0, 5, 6, 7, 8, 9, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 28, 29} {2, 3, 4, 5, 8, 11, 12, 13, 18, 19, 20, 21, 22, 24, 25, 26, 29, 30} {1, 6, 7, 8, 10, 11, 12, 14, 16, 17, 22, 24, 25, 26, 27, 28} t17 cert17 h17
  obtain ⟨t19, h19⟩ := chain_step {2, 3, 4, 5, 8, 11, 12, 13, 18, 19, 20, 21, 22, 24, 25, 26, 29, 30} {5, 7, 8, 10, 13, 14, 16, 17, 19, 20, 22, 24, 27, 30} {8, 12, 13, 14, 15, 19, 21, 22, 24, 26, 27, 28, 29, 30} t18 cert18 h18
  obtain ⟨t20, h20⟩ := chain_step {5, 7, 8, 10, 13, 14, 16, 17, 19, 20, 22, 24, 27, 30} {4, 6, 7, 8, 9, 13, 16, 19, 20, 23, 25, 28} {4, 5, 6, 12, 13, 23, 24, 26, 29, 30} t19 cert19 h19
  obtain ⟨t21, h21⟩ := chain_step {4, 6, 7, 8, 9, 13, 16, 19, 20, 23, 25, 28} {1, 2, 3, 4, 9, 16, 19, 20, 21, 22, 23, 27, 28, 29, 30} {1, 5, 7, 9, 10, 12, 13, 14, 15, 16, 19, 20, 22, 25, 26} t20 cert20 h20
  obtain ⟨t22, h22⟩ := chain_step {1, 2, 3, 4, 9, 16, 19, 20, 21, 22, 23, 27, 28, 29, 30} {0, 2, 5, 6, 7, 12, 14, 15, 20, 21, 23, 24, 25, 29, 30} {0, 1, 2, 4, 9, 10, 11, 16, 17, 22, 23, 25, 27, 28, 29, 30} t21 cert21 h21
  obtain ⟨t23, h23⟩ := chain_step {0, 2, 5, 6, 7, 12, 14, 15, 20, 21, 23, 24, 25, 29, 30} {0, 2, 3, 6, 8, 9, 10, 13, 16, 17, 19, 22, 26, 28, 30} {0, 4, 5, 6, 7, 9, 12, 16, 18, 19, 24, 26, 27, 28, 29, 30} t22 cert22 h22
  obtain ⟨t24, h24⟩ := chain_step {0, 2, 3, 6, 8, 9, 10, 13, 16, 17, 19, 22, 26, 28, 30} {0, 1, 2, 5, 6, 8, 11, 12, 14, 16, 18, 21, 22, 27, 30} {0, 1, 2, 3, 7, 10, 11, 12, 14, 16, 17, 20, 21, 25, 29, 30} t23 cert23 h23
  obtain ⟨t25, h25⟩ := chain_step {0, 1, 2, 5, 6, 8, 11, 12, 14, 16, 18, 21, 22, 27, 30} {0, 7, 9, 10, 11, 12, 13, 15, 16, 17, 19, 21, 23, 24, 27, 28} {0, 2, 3, 4, 5, 7, 8, 9, 10, 11, 13, 14, 17, 18, 23, 24, 26, 29, 30} t24 cert24 h24
  obtain ⟨t26, h26⟩ := chain_step {0, 7, 9, 10, 11, 12, 13, 15, 16, 17, 19, 21, 23, 24, 27, 28} {0, 1, 2, 3, 4, 6, 12, 13, 14, 17, 21, 23, 24, 27, 28} {0, 1, 2, 5, 10, 12, 13, 14, 15, 17, 18, 20, 22, 23, 24, 25, 26} t25 cert25 h25
  obtain ⟨t27, h27⟩ := chain_step {0, 1, 2, 3, 4, 6, 12, 13, 14, 17, 21, 23, 24, 27, 28} {1, 3, 4, 5, 9, 11, 12, 14, 15, 16, 17, 20, 21, 22, 26, 30} {4, 5, 7, 8, 10, 12, 13, 14, 15, 17, 18, 20, 22, 23, 24, 25, 26} t26 cert26 h26
  obtain ⟨t28, h28⟩ := chain_step {1, 3, 4, 5, 9, 11, 12, 14, 15, 16, 17, 20, 21, 22, 26, 30} {2, 4, 6, 7, 8, 9, 11, 15, 20, 26, 29, 30} {2, 5, 6, 7, 9, 10, 13, 17, 20, 21, 22, 26, 29, 30} t27 cert27 h27
  obtain ⟨t29, h29⟩ := chain_step {2, 4, 6, 7, 8, 9, 11, 15, 20, 26, 29, 30} {4, 6, 7, 9, 10, 12, 13, 14, 16, 19, 20, 21, 22, 25, 26, 27, 28, 29, 30} {4, 8, 11, 14, 15, 16, 17, 18, 20, 21, 22, 24, 26, 27, 28, 29, 30} t28 cert28 h28
  obtain ⟨t30, h30⟩ := chain_step {4, 6, 7, 9, 10, 12, 13, 14, 16, 19, 20, 21, 22, 25, 26, 27, 28, 29, 30} {0, 2, 3, 6, 7, 9, 10, 11, 13, 17, 18, 21, 23, 24, 25, 27, 28, 30} {0, 1, 2, 4, 7, 8, 10, 11, 15, 16, 17, 18, 19, 20, 21, 23, 25, 27, 28, 29, 30} t29 cert29 h29
  obtain ⟨t31, h31⟩ := chain_step {0, 2, 3, 6, 7, 9, 10, 11, 13, 17, 18, 21, 23, 24, 25, 27, 28, 30} {0} {0, 2, 5, 7, 8, 9, 11, 12, 13, 15, 16, 19, 20, 22, 23, 24, 25, 29, 30} t30 cert30 h30
  have hexp : ((2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * (2 * 1 + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) : ℕ) = 2147483647 := by omega
  rw [hexp] at h31
  exact ⟨t31, h31⟩

/-! ### Corpo quociente GF(2)[X]/(P31) e a ordem de X -/

def I31 : Ideal (Polynomial (ZMod 2)) := Ideal.span {P31}

abbrev Q31 := Polynomial (ZMod 2) ⧸ I31

def u : Q31 := Ideal.Quotient.mk I31 X

theorem N_prime : Nat.Prime 2147483647 := by native_decide

theorem mk_P31_zero : Ideal.Quotient.mk I31 P31 = 0 :=
  Ideal.Quotient.eq_zero_iff_mem.mpr (Ideal.mem_span_singleton_self P31)

theorem one_add_one_poly : (1 : Polynomial (ZMod 2)) + 1 = 0 := by
  rw [← Polynomial.C_1, ← Polynomial.C_add,
      show (1 : ZMod 2) + 1 = 0 from by decide, Polynomial.C_0]

theorem one_add_one_Q31 : (1 : Q31) + 1 = 0 := by
  have h0 : Ideal.Quotient.mk I31 ((1 : Polynomial (ZMod 2)) + (1 : Polynomial (ZMod 2)))
      = 0 := by
    have h := congrArg (Ideal.Quotient.mk I31) one_add_one_poly
    rwa [map_zero] at h
  have h1 : (1 : Q31) + (1 : Q31)
      = Ideal.Quotient.mk I31 ((1 : Polynomial (ZMod 2)) + (1 : Polynomial (ZMod 2))) :=
    (map_add (Ideal.Quotient.mk I31) (1 : Polynomial (ZMod 2)) (1 : Polynomial (ZMod 2))).symm
  rw [h1]
  exact h0

/-- A identidade da cadeia dá u^N = 1. -/
theorem u_pow_N : u ^ 2147483647 = 1 := by
  obtain ⟨t, ht⟩ := chain_full
  have ht' := ht
  rw [polyOf_zero_set] at ht'
  have hm : Ideal.Quotient.mk I31 (X ^ 2147483647 + 1)
      = Ideal.Quotient.mk I31 (P31 * t) := by rw [ht']
  rw [map_add, map_pow, map_one, map_mul, mk_P31_zero, zero_mul] at hm
  have hm2 : u ^ 2147483647 + 1 = 0 := hm
  have hkey : u ^ 2147483647 + 1 + 1 = u ^ 2147483647 := by
    rw [add_assoc, one_add_one_Q31, add_zero]
  have hval : u ^ 2147483647 + 1 + 1 = 1 := by rw [hm2, zero_add]
  exact hkey.symm.trans hval

theorem u_ne_one : u ≠ 1 := by
  intro h
  have h' : Ideal.Quotient.mk I31 X
      = Ideal.Quotient.mk I31 (1 : Polynomial (ZMod 2)) := h
  have hmem : X - (1 : Polynomial (ZMod 2)) ∈ I31 := Ideal.Quotient.eq.mp h'
  have hdvd : P31 ∣ X - (1 : Polynomial (ZMod 2)) := Ideal.mem_span_singleton.mp hmem
  have hneg1 : (-1 : Polynomial (ZMod 2)) = 1 :=
    neg_eq_of_add_eq_zero_left one_add_one_poly
  have hdvd2 : P31 ∣ X + 1 := by
    have hX1 : X - (1 : Polynomial (ZMod 2)) = X + 1 := by
      rw [sub_eq_add_neg, hneg1]
    rw [hX1] at hdvd
    exact hdvd
  have hne : (X + 1 : Polynomial (ZMod 2)) ≠ 0 := by
    intro h0
    have hc : (X + 1 : Polynomial (ZMod 2)).coeff 1 = 0 := by rw [h0]; simp
    rw [Polynomial.coeff_add, Polynomial.coeff_X_one, Polynomial.coeff_one] at hc
    exact absurd hc (by decide)
  have hdeg := Polynomial.natDegree_le_of_dvd hdvd2 hne
  rw [P31_natDegree] at hdeg
  have hupper : (X + 1 : Polynomial (ZMod 2)).natDegree ≤ 1 := by
    refine le_trans (Polynomial.natDegree_add_le _ _) ?_
    simp [Polynomial.natDegree_X, Polynomial.natDegree_one]
  omega

/-- Ordem de um elemento: o mínimo expoente positivo redivide qualquer outro. -/
theorem order_dvd {M : Type*} [Monoid M] {v : M} {o d : ℕ}
    (ho : 0 < o) (hvo : v ^ o = 1)
    (hmin : ∀ k, 0 < k → k < o → v ^ k ≠ 1)
    (hd : v ^ d = 1) : o ∣ d := by
  obtain ⟨q, r, hqr, hrlt⟩ : ∃ q r, d = o * q + r ∧ r < o :=
    ⟨d / o, d % o, (Nat.div_add_mod d o).symm, Nat.mod_lt _ ho⟩
  have hd' : v ^ r = 1 := by
    have h3 : v ^ (o * q + r) = 1 := by rw [← hqr]; exact hd
    rw [pow_add, pow_mul, hvo, one_pow, one_mul] at h3
    exact h3
  rcases Nat.eq_zero_or_pos r with hr | hr
  · exact ⟨q, by omega⟩
  · exact absurd hd' (hmin r hr hrlt)

/-- A ordem de u é exatamente N = 2^31-1 (primo). -/
theorem u_pow_ne_one_of_lt {d : ℕ} (h0 : 0 < d) (hlt : d < 2147483647) : u ^ d ≠ 1 := by
  intro hd1
  have hne : {k : ℕ | 0 < k ∧ u ^ k = 1}.Nonempty := ⟨2147483647, by omega, u_pow_N⟩
  obtain ⟨ho, huo⟩ := Nat.sInf_mem hne
  have hmin : ∀ k, 0 < k → k < sInf {k : ℕ | 0 < k ∧ u ^ k = 1} → u ^ k ≠ 1 := by
    intro k hk0 hko huk
    have hmem : k ∈ {k : ℕ | 0 < k ∧ u ^ k = 1} := ⟨hk0, huk⟩
    have := Nat.sInf_le hmem
    omega
  have hdvdN : sInf {k : ℕ | 0 < k ∧ u ^ k = 1} ∣ 2147483647 :=
    order_dvd ho huo hmin u_pow_N
  obtain ⟨k, hk⟩ := hdvdN
  have hk1 : k = 1 := by
    by_contra hk1'
    have hso : sInf {k : ℕ | 0 < k ∧ u ^ k = 1} ≠ 1 := by
      intro h1o
      rw [h1o, pow_one] at huo
      exact u_ne_one huo
    have hk0 : k ≠ 0 := by
      intro hk0
      rw [hk0, mul_zero] at hk
      omega
    exact absurd (show Nat.Prime (sInf {k : ℕ | 0 < k ∧ u ^ k = 1} * k) from by
      rw [← hk]
      exact N_prime) (Nat.not_prime_mul (by omega) (by omega))
  rw [hk1] at hk
  have hoN : sInf {k : ℕ | 0 < k ∧ u ^ k = 1} = 2147483647 := by omega
  have hdvd : sInf {k : ℕ | 0 < k ∧ u ^ k = 1} ∣ d := order_dvd ho huo hmin hd1
  obtain ⟨q, hq⟩ := hdvd
  rcases Nat.eq_zero_or_pos q with hq0 | hq1
  · rw [hq0, mul_zero] at hq
    omega
  · have hle : 2147483647 ≤ d := by
      rw [hq, hoN]
      calc 2147483647 = 2147483647 * 1 := (mul_one _).symm
        _ ≤ 2147483647 * q := Nat.mul_le_mul_left _ (by omega)
    omega

/-- Coeficientes constantes não-nulos, na forma ≠ 0 exigida pelas pontes. -/
theorem P31_coeff_0_ne : P31.coeff 0 ≠ 0 := by
  rw [P31_coeff_0]; exact one_ne_zero

theorem GG_coeff_0_ne : GG.coeff 0 ≠ 0 := by
  rw [GG_coeff_0]; exact one_ne_zero

/-! ### Pontes de divisibilidade -/

theorem P31_dvd_of_GG_dvd {E : Polynomial (ZMod 2)} (h : GG ∣ E) : P31 ∣ E := by
  obtain ⟨t, ht⟩ := h
  refine ⟨(X + 1 : Polynomial (ZMod 2)) * t, ?_⟩
  rw [← GG_eq] at ht
  linear_combination ht

/-- Fator constante não-nulo: X pode ser cancelado de uma divisibilidade. -/
theorem dvd_of_dvd_X_mul {H B : Polynomial (ZMod 2)} (h0 : H.coeff 0 ≠ 0)
    (h : H ∣ X * B) : H ∣ B := by
  obtain ⟨q, hq⟩ := h
  have hH0 : H.eval 0 ≠ 0 := by
    rw [← Polynomial.coeff_zero_eq_eval_zero]
    exact h0
  have hc0 : H.eval 0 * q.eval 0 = 0 := by
    rw [← Polynomial.eval_mul, ← hq, Polynomial.eval_mul, Polynomial.eval_X, zero_mul]
  have hq0 : q.eval 0 = 0 := by
    by_contra hne
    exact absurd hc0 (mul_ne_zero hH0 hne)
  have hq0c : q.coeff 0 = 0 := by
    rw [Polynomial.coeff_zero_eq_eval_zero]
    exact hq0
  obtain ⟨q', hq'⟩ : X ∣ q := Polynomial.X_dvd_iff.mpr hq0c
  refine ⟨q', ?_⟩
  refine mul_left_cancel₀ (Polynomial.X_ne_zero : (X : Polynomial (ZMod 2)) ≠ 0) ?_
  calc X * B = H * q := hq
    _ = H * (X * q') := by rw [hq']
    _ = X * (H * q') := by ring

theorem dvd_of_dvd_X_pow_mul {H : Polynomial (ZMod 2)} (h0 : H.coeff 0 ≠ 0) :
    ∀ (a : ℕ) {B : Polynomial (ZMod 2)}, H ∣ X ^ a * B → H ∣ B := by
  intro a
  induction a with
  | zero => intro B hb; rwa [pow_zero, one_mul] at hb
  | succ a ih =>
    intro B hb
    rw [pow_succ', mul_assoc] at hb
    exact ih (dvd_of_dvd_X_mul h0 hb)

/-- Ponte quociente: P31 ∣ X^d + 1 implica u^d = 1. -/
theorem u_pow_eq_one_of_dvd {d : ℕ} (h : P31 ∣ X ^ d + 1) : u ^ d = 1 := by
  obtain ⟨t, ht⟩ := h
  have hm : Ideal.Quotient.mk I31 (X ^ d + 1) = Ideal.Quotient.mk I31 (P31 * t) := by
    rw [ht]
  rw [map_add, map_pow, map_one, map_mul, mk_P31_zero, zero_mul] at hm
  have hm2 : u ^ d + 1 = 0 := hm
  have hkey : u ^ d + 1 + 1 = u ^ d := by
    rw [add_assoc, one_add_one_Q31, add_zero]
  have hval : u ^ d + 1 + 1 = 1 := by rw [hm2, zero_add]
  exact hkey.symm.trans hval

/-- Span < N: P31 não divide X^d + 1 (a ordem de X é o primo N). -/
theorem P31_not_dvd_X_pow_add_one {d : ℕ} (h0 : 0 < d) (hlt : d < 2147483647) :
    ¬ P31 ∣ X ^ d + 1 := by
  intro h
  exact u_pow_ne_one_of_lt h0 hlt (u_pow_eq_one_of_dvd h)

/-! ### Teoremas de classe de detecção -/

/-- Burst <= 32 bits: E = X^a·B com grau(B) <= 31 e B ≠ 0 é sempre detectado. -/
theorem crc32c_burst_detected {a : ℕ} {B : Polynomial (ZMod 2)}
    (hB : B.natDegree ≤ 31) (hB0 : B ≠ 0) : ¬ GG ∣ X ^ a * B := by
  intro hdvd
  have hBB : GG ∣ B := dvd_of_dvd_X_pow_mul GG_coeff_0_ne a hdvd
  have h32 : GG.natDegree ≤ B.natDegree := Polynomial.natDegree_le_of_dvd hBB hB0
  rw [GG_natDegree] at h32
  omega

/-- 2-bit com span < 2^31-1: X^a + X^b (a < b < N) é sempre detectado. -/
theorem crc32c_two_bit_detected {a b : ℕ} (hab : a < b) (hb : b < 2147483647) :
    ¬ GG ∣ X ^ a + X ^ b := by
  intro hdvd
  have hP := P31_dvd_of_GG_dvd hdvd
  have hSplit : (X ^ a + X ^ b : Polynomial (ZMod 2)) = X ^ a * (1 + X ^ (b - a)) := by
    rw [mul_add, mul_one, ← pow_add, Nat.add_sub_cancel' (Nat.le_of_lt hab)]
  rw [hSplit] at hP
  have h1 : P31 ∣ X ^ (b - a) + 1 := by
    have hz := dvd_of_dvd_X_pow_mul P31_coeff_0_ne a hP
    rwa [add_comm] at hz
  exact P31_not_dvd_X_pow_add_one (Nat.sub_pos_of_lt hab) (by omega) h1

theorem smul_one_eq_cast' (n : ℕ) : n • (1 : ZMod 2) = (n : ZMod 2) := by
  induction n with
  | zero => simp
  | succ n ih => rw [succ_nsmul, ih, Nat.cast_succ]

theorem eval_polyOf_one (S : Finset ℕ) :
    (polyOf S).eval (1 : ZMod 2) = (S.card : ZMod 2) := by
  induction S using Finset.induction_on with
  | empty => simp [polyOf]
  | insert x S hx ih =>
    rw [polyOf_insert x S hx, Polynomial.eval_add, ih,
      Polynomial.eval_pow, Polynomial.eval_X, one_pow,
      Finset.card_insert_of_notMem hx, Nat.cast_add, Nat.cast_one]
    ring

/-- Peso ímpar: qualquer polinômio com número ímpar de termos é detectado. -/
theorem crc32c_odd_weight_detected (S : Finset ℕ) (hodd : S.card % 2 = 1) :
    ¬ GG ∣ polyOf S := by
  intro hdvd
  obtain ⟨t, ht⟩ := hdvd
  have h1 : (polyOf S).eval (1 : ZMod 2) = (GG * t).eval (1 : ZMod 2) := by rw [ht]
  rw [eval_polyOf_one, Polynomial.eval_mul, ← GG_eq, Polynomial.eval_mul,
      Polynomial.eval_add, Polynomial.eval_X, Polynomial.eval_one] at h1
  have h11 : (1 : ZMod 2) + 1 = 0 := by decide
  rw [h11, zero_mul, zero_mul] at h1
  obtain ⟨m, hm⟩ : ∃ m, S.card = 2 * m + 1 := ⟨S.card / 2, by omega⟩
  rw [hm, Nat.cast_add, Nat.cast_mul,
    show ((2 : ℕ) : ZMod 2) = 0 from by decide, zero_mul, zero_add,
    Nat.cast_one] at h1
  exact zero_ne_one h1.symm

/-- Peso <= 3 com expoentes < N: capstone combinando 1-bit, 2-bit e ímpar. -/
theorem crc32c_weight_le_three_detected (S : Finset ℕ) (hS : S.card ≤ 3)
    (hlt : ∀ i ∈ S, i < 2147483647) (hE : polyOf S ≠ 0) : ¬ GG ∣ polyOf S := by
  rcases (by omega : S.card = 0 ∨ S.card = 1 ∨ S.card = 2 ∨ S.card = 3) with h | h | h | h
  · rw [Finset.card_eq_zero.mp h] at hE
    exact absurd polyOf_empty hE
  · obtain ⟨a, ha⟩ := Finset.card_eq_one.mp h
    intro hdvd
    rw [ha, polyOf_singleton] at hdvd
    have h1 : GG ∣ X ^ a * 1 := by rwa [mul_one]
    have hG1 : GG ∣ 1 := dvd_of_dvd_X_pow_mul GG_coeff_0_ne a h1
    have hdeg := Polynomial.natDegree_le_of_dvd hG1 one_ne_zero
    rw [GG_natDegree, Polynomial.natDegree_one] at hdeg
    omega
  · obtain ⟨a, b, hab, hSet⟩ := Finset.card_eq_two.mp h
    intro hdvd
    rw [hSet, polyOf_pair a b hab] at hdvd
    rcases Nat.lt_or_ge a b with hltab | hge
    · exact crc32c_two_bit_detected hltab (hlt b (by rw [hSet]; simp)) hdvd
    · have hba : b < a := by omega
      have hdvd' : GG ∣ X ^ b + X ^ a := by rw [add_comm]; exact hdvd
      exact crc32c_two_bit_detected hba (hlt a (by rw [hSet]; simp)) hdvd'
  · exact crc32c_odd_weight_detected S (by omega)

/-- Palavra-código + erro detectável permanece detectável. -/
theorem crc32c_codeword_error_detected {T : Polynomial (ZMod 2)} {S : Finset ℕ}
    (hT : GG ∣ T) (hdet : ¬ GG ∣ polyOf S) : ¬ GG ∣ T + polyOf S := by
  intro h
  have h2 : GG ∣ polyOf S := by
    have hz := dvd_sub h hT
    rwa [add_sub_cancel_left] at hz
  exact hdet h2

end
