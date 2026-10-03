-- Theorems over the Aeneas extract of production isolated_kernel.rs (F83).
-- Lengths are always `x.val.length : Nat`. Loops use `loop.spec_decr_nat`
-- + `step` on `index_usize_spec`. Do not `simp [loop]`.
import Aeneas
import IsolatedKernel
open Aeneas.Std Result
open Aeneas.Std.WP
open pedra_aeneas_isolated_kernel

/-- F83 closed form. -/
def isolated_ok (key id : Aeneas.Std.Slice U8) : Prop :=
  key.val.take id.val.length = id.val ∧
    (key.val.length = id.val.length ∨
      ∃ _ : id.val.length < key.val.length,
        key.val[id.val.length] = ISOLATED_CHILD_SEP)

def isolated_as_is_ok (key id : Aeneas.Std.Slice U8) : Prop :=
  key.val.take id.val.length = id.val

private theorem take_ne_of_at {α} {k d : List α} {j n : Nat}
    (hj : j < n) (hk : j < k.length) (hn : n ≤ d.length)
    (hne : k[j] ≠ d[j]) : k.take n ≠ d := by
  intro heq
  have hj' : j < (k.take n).length := by simp [List.length_take]; omega
  have htk : (k.take n)[j] = k[j] := List.getElem_take
  have htd : (k.take n)[j] = d[j] := by
    have : j < d.length := by omega
    simp [heq]
  exact hne (htk.symm.trans htd)

private theorem take_succ {α} {k d : List α} {j : Nat}
    (hk : j < k.length) (hd : j < d.length)
    (hpref : k.take j = d.take j) (heq : k[j] = d[j]) :
    k.take (j + 1) = d.take (j + 1) := by
  apply List.ext_getElem
  · simp [List.length_take]; omega
  · intro t ht _
    have ht1 : t < j + 1 := by
      simp [List.length_take] at ht; omega
    have htk : t < (k.take (j + 1)).length := ht
    have htd : t < (d.take (j + 1)).length := by
      simp [List.length_take] at ht ⊢; omega
    rw [List.getElem_take (h := htk), List.getElem_take (h := htd)]
    cases Nat.eq_or_lt_of_le (Nat.le_of_lt_succ ht1) with
    | inr htj =>
      have hlen : t < (k.take j).length := by simp [List.length_take]; omega
      have := List.getElem_of_eq hpref hlen
      simpa [List.getElem_take] using this
    | inl hte =>
      cases hte
      exact heq

theorem isolated_id_matches_loop_spec
    (key id : Aeneas.Std.Slice U8) (i : Usize)
    (hInv :
      i.val ≤ id.val.length ∧
      id.val.length ≤ key.val.length ∧
      key.val.take i.val = id.val.take i.val) :
    spec (isolated_id_matches_loop key id i)
      (fun b => (b = true) ↔ isolated_ok key id) := by
  unfold isolated_id_matches_loop
  refine
    loop.spec_decr_nat
      (fun j : Usize => id.val.length - j.val)
      (fun j =>
        j.val ≤ id.val.length ∧
        id.val.length ≤ key.val.length ∧
        key.val.take j.val = id.val.take j.val)
      (fun b => (b = true) ↔ isolated_ok key id)
      (isolated_id_matches_loop.body key id) i ?body hInv
  intro j ⟨hj_le, hj_key, hj_pref⟩
  unfold isolated_id_matches_loop.body
  dsimp +zeta only
  split
  · -- j < Slice.len id
    rename_i hltU
    have hlt : j.val < id.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hltU
    have hjk : j.val < key.val.length := Nat.lt_of_lt_of_le hlt hj_key
    step as ⟨ kb, hkb ⟩
    step as ⟨ ib, hib ⟩
    split
    · -- mismatch → ¬ isolated_ok
      rename_i hneU
      have hne : kb ≠ ib := by
        intro he
        simp [he, bne_iff_ne] at hneU
      have hne' : key.val[j.val] ≠ id.val[j.val] := by
        simpa [hkb, hib] using hne
      simp [spec_ok]
      intro hok
      apply take_ne_of_at (k := key.val) (d := id.val) (j := j.val)
        (n := id.val.length) hlt hjk (Nat.le_refl _) hne' hok.1
    · -- bytes equal: continue
      rename_i heqU
      have heq : kb = ib := UScalar.eq_imp _ _ (by simpa using heqU)
      have heq' : key.val[j.val] = id.val[j.val] := by
        simpa [hkb, hib] using heq
      step as ⟨ j', hj' ⟩
      have hjv : (↑j' : Nat) = (↑j : Nat) + 1 := by simpa using hj'
      refine ⟨?le, hj_key, ?pref, ?meas⟩
      · omega
      · have : key.val.take (j.val + 1) = id.val.take (j.val + 1) :=
          take_succ hjk hlt hj_pref heq'
        simpa [hjv]
      · omega
  · -- j ≥ Slice.len id
    rename_i hgeU
    have hge : ¬ j.val < id.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hgeU
    have hj_eq : j.val = id.val.length := Nat.le_antisymm hj_le (Nat.not_lt.mp hge)
    split
    · -- equal lengths
      rename_i heqU
      have heqLen : key.val.length = id.val.length := by
        simpa [Aeneas.Std.Slice.len_val] using congrArg (fun x : Usize => x.val) heqU
      simp [spec_ok]
      refine ⟨?_, Or.inl heqLen⟩
      simpa [hj_eq] using hj_pref
    · -- longer key: check separator
      rename_i hneU
      have hneLen : key.val.length ≠ id.val.length := by
        intro h
        exact hneU (UScalar.eq_imp _ _ (by simpa [Aeneas.Std.Slice.len_val] using h))
      have hltK : id.val.length < key.val.length :=
        Nat.lt_of_le_of_ne hj_key hneLen.symm
      have hji : j.val < key.val.length := by simp [hj_eq]; exact hltK
      step as ⟨ next, hnext ⟩
      simp
      constructor
      · intro hb
        refine ⟨?_, Or.inr ⟨hltK, ?_⟩⟩
        · simpa [hj_eq] using hj_pref
        · have hidx : (key.val)[j.val] = (key.val)[id.val.length] := by
            simp [hj_eq]
          exact hidx.symm.trans (hnext.symm.trans hb)
      · intro hok
        cases hok.2 with
        | inl he => exact (hneLen he).elim
        | inr hex =>
          obtain ⟨_, hsep⟩ := hex
          have : next = key.val[id.val.length] := by
            simpa [hj_eq] using hnext
          simpa [this, hsep]

theorem isolated_id_matches_spec
    (key id : Aeneas.Std.Slice U8) (hlen : id.val.length ≤ key.val.length) :
    spec (isolated_id_matches key id)
      (fun b => (b = true) ↔ isolated_ok key id) := by
  unfold isolated_id_matches
  dsimp +zeta only
  split
  · rename_i hlt
    have : key.val.length < id.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hlt
    omega
  · apply isolated_id_matches_loop_spec
    simp [hlen]

theorem isolated_id_matches_as_is_loop_spec
    (key id : Aeneas.Std.Slice U8) (i : Usize)
    (hInv :
      i.val ≤ id.val.length ∧
      id.val.length ≤ key.val.length ∧
      key.val.take i.val = id.val.take i.val) :
    spec (isolated_id_matches_as_is_loop key id i)
      (fun b => (b = true) ↔ isolated_as_is_ok key id) := by
  unfold isolated_id_matches_as_is_loop
  refine
    loop.spec_decr_nat
      (fun j : Usize => id.val.length - j.val)
      (fun j =>
        j.val ≤ id.val.length ∧
        id.val.length ≤ key.val.length ∧
        key.val.take j.val = id.val.take j.val)
      (fun b => (b = true) ↔ isolated_as_is_ok key id)
      (isolated_id_matches_as_is_loop.body key id) i ?body hInv
  intro j ⟨hj_le, hj_key, hj_pref⟩
  unfold isolated_id_matches_as_is_loop.body
  dsimp +zeta only
  split
  · rename_i hltU
    have hlt : j.val < id.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hltU
    have hjk : j.val < key.val.length := Nat.lt_of_lt_of_le hlt hj_key
    step as ⟨ kb, hkb ⟩
    step as ⟨ ib, hib ⟩
    split
    · rename_i hneU
      have hne : kb ≠ ib := by
        intro he
        simp [he] at hneU
      simp [spec_ok]
      intro hok
      have hne' : key.val[j.val] ≠ id.val[j.val] := by
        simpa [hkb, hib] using hne
      exact take_ne_of_at hlt hjk (Nat.le_refl _) hne' hok
    · rename_i heqU
      have heq : kb = ib := UScalar.eq_imp _ _ (by simpa using heqU)
      have heq' : key.val[j.val] = id.val[j.val] := by
        simpa [hkb, hib] using heq
      step as ⟨ j', hj' ⟩
      have hjv : (↑j' : Nat) = (↑j : Nat) + 1 := by simpa using hj'
      refine ⟨?le, hj_key, ?pref, ?meas⟩
      · omega
      · have : key.val.take (j.val + 1) = id.val.take (j.val + 1) :=
          take_succ hjk hlt hj_pref heq'
        simpa [hjv]
      · omega
  · rename_i hgeU
    have hge : ¬ j.val < id.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hgeU
    have hj_eq : j.val = id.val.length := Nat.le_antisymm hj_le (Nat.not_lt.mp hge)
    simp [spec_ok]
    simpa [isolated_as_is_ok, hj_eq] using hj_pref

theorem isolated_id_matches_as_is_spec
    (key id : Aeneas.Std.Slice U8) (hlen : id.val.length ≤ key.val.length) :
    spec (isolated_id_matches_as_is key id)
      (fun b => (b = true) ↔ isolated_as_is_ok key id) := by
  unfold isolated_id_matches_as_is
  dsimp +zeta only
  split
  · rename_i hlt
    have : key.val.length < id.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hlt
    omega
  · apply isolated_id_matches_as_is_loop_spec
    simp [hlen]

theorem isolated_child_byte_slash :
    isolated_child_byte ISOLATED_CHILD_SEP = ok true := by
  simp [isolated_child_byte]

theorem isolated_child_byte_as_is_always (b : U8) :
    isolated_child_byte_as_is b = ok true := by
  simp [isolated_child_byte_as_is]

theorem isolated_id_matches_too_short
    (key id : Aeneas.Std.Slice U8)
    (h : key.val.length < id.val.length) :
    isolated_id_matches key id = ok false
    ∧ isolated_id_matches_as_is key id = ok false := by
  have hlt : Aeneas.Std.Slice.len key < Aeneas.Std.Slice.len id := by
    simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using h
  constructor
  · unfold isolated_id_matches; simp [hlt]
  · unfold isolated_id_matches_as_is; simp [hlt]

private def sbytes (l : List U8) (h : l.length ≤ Usize.max := by scalar_tac) :
    Aeneas.Std.Slice U8 :=
  ⟨l, h⟩

private def idVmA : Aeneas.Std.Slice U8 :=
  sbytes [47#u8, 118#u8, 109#u8, 47#u8, 118#u8, 109#u8, 45#u8, 97#u8]

private def keyVmAB : Aeneas.Std.Slice U8 :=
  sbytes [47#u8, 118#u8, 109#u8, 47#u8, 118#u8, 109#u8, 45#u8, 97#u8, 98#u8]

/-- F83: FIXED drops the sibling; AS-IS leaks it. -/
theorem as_is_leaks_sibling :
    isolated_id_matches keyVmAB idVmA = ok false
    ∧ isolated_id_matches_as_is keyVmAB idVmA = ok true := by
  have hlen : idVmA.val.length ≤ keyVmAB.val.length := by
    simp [idVmA, keyVmAB, sbytes]
  have hf := isolated_id_matches_spec keyVmAB idVmA hlen
  have ha := isolated_id_matches_as_is_spec keyVmAB idVmA hlen
  have hspecF : ¬ isolated_ok keyVmAB idVmA := by
    simp [isolated_ok, idVmA, keyVmAB, sbytes, ISOLATED_CHILD_SEP]
  have hspecA : isolated_as_is_ok keyVmAB idVmA := by
    simp [isolated_as_is_ok, idVmA, keyVmAB, sbytes]
  cases hfm : isolated_id_matches keyVmAB idVmA with
  | ok b =>
    cases ham : isolated_id_matches_as_is keyVmAB idVmA with
    | ok c =>
      simp [spec, theta, wp_return, hfm] at hf
      simp [spec, theta, wp_return, ham] at ha
      have hb : b = false := Bool.eq_false_iff.2 fun ht => hspecF (hf.mp ht)
      have hc : c = true := ha.mpr hspecA
      simp [hb, hc]
    | fail _ => simp [spec, theta, ham] at ha
    | div => simp [spec, theta, ham] at ha
  | fail _ => simp [spec, theta, hfm] at hf
  | div => simp [spec, theta, hfm] at hf

/-- RFC-0218 P2.1 7/12 (átomo `catalog:isolated_child`, entrada
    `isolated_child_byte`): depois de um id exato, o byte de
    continuação é filho EXATAMENTE quando é a barra citada
    ISOLATED_CHILD_SEP. O AS-IS aceita qualquer byte (irmão vira
    filho — dente plantado). -/
theorem isolated_child_byte_fate_iff :
    ∀ (next : U8) (v : Bool),
      (isolated_child_byte next = ok v) ↔
      (v = decide (next = ISOLATED_CHILD_SEP)) := by
  intro next v
  constructor
  · intro hval
    unfold isolated_child_byte at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P2.1 8/12 (átomo `catalog:isolated`, entrada
    `isolated_id_matches`): o id isolado casa EXATAMENTE na guarda
    citada de tamanho — chave menor que o id nunca casa (false);
    com tamanho suficiente, a decisão é o loop citado
    `isolated_id_matches_loop` (byte a byte, fronteira em
    ISOLATED_CHILD_SEP — corpo do loop não reaberto neste degrau).
    O AS-IS casa por prefixo (irmão vira filho — dente plantado). -/
theorem isolated_id_matches_fate_iff :
    ∀ (key id : Slice U8) (v : Bool),
      (isolated_id_matches key id = ok v) ↔
        ((key.len < id.len ∧ v = false) ∨
         (¬ (key.len < id.len) ∧
            isolated_id_matches_loop key id 0#usize = ok v)) := by
  intro key id v
  constructor
  · intro hval
    unfold isolated_id_matches at hval
    dsimp only at hval
    split at hval
    · next hc =>
      injection hval with hv
      exact Or.inl ⟨hc, hv.symm⟩
    · next hc =>
      exact Or.inr ⟨hc, hval⟩
  · rintro (⟨hc, hv⟩ | ⟨hc, hv⟩)
    · subst hv
      unfold isolated_id_matches
      dsimp only
      rw [if_pos hc]
    · unfold isolated_id_matches
      dsimp only
      rw [if_neg (by simp [hc])]
      exact hv
