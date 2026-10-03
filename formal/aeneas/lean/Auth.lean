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

/-- Any ok-valued Result bind forces the bound term to be ok. -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- An ok chain reassembles into an ok bind. -/
private theorem bind_intro {α β} {x : Result α} {f : α → Result β} {v : β}
    (a : α) (hx : x = ok a) (h : f a = ok v) : Aeneas.Std.bind x f = ok v := by
  rw [hx]
  exact h

/- RFC-0202 P1.2 (fifth registered close): the whole-output fate of the
    `bearer` catalog entry (`bearer_token_from_value`, the extractor the
    live authorize handler calls) is decided EXACTLY along its callee
    chain — trim, emptiness, first-whitespace split, then the two
    pedra-local scheme gates (is_bearer_scheme / is_non_bearer_auth_
    scheme). Every none/some fate on the RHS pins which callee answered
    what; the core.str primitives stay axiom-shaped (opaque), the two
    scheme gates are the composed extract callees. The RFC's first
    candidate (group_validate, N-way lost-update step) measured refusal
    — extracted as partial_fixpoint, irreducible to defeq — and fell to
    this board pair per the 0200 P1.2 re-scope cadence. -/
theorem bearer_token_from_value_fate_iff :
    ∀ (value : Str) (t : Option Str),
      (bearer_token_from_value value = ok t) ↔
        ∃ v, core.str.Str.trim value = ok v ∧
          ((core.str.Str.is_empty v = ok true ∧ t = none) ∨
           (core.str.Str.is_empty v = ok false ∧
             ((∃ scheme rest,
                 core.str.Str.split_once_ws v = ok (some (scheme, rest)) ∧
                 ((is_bearer_scheme scheme = ok true ∧
                     ∃ tok, core.str.Str.trim rest = ok tok ∧
                       ((core.str.Str.is_empty tok = ok true ∧ t = none) ∨
                        (core.str.Str.is_empty tok = ok false ∧ t = some tok))) ∨
                  (is_bearer_scheme scheme = ok false ∧ t = none))) ∨
              (core.str.Str.split_once_ws v = ok none ∧
                 ((is_bearer_scheme v = ok true ∧ t = none) ∨
                  (is_bearer_scheme v = ok false ∧
                     ((is_non_bearer_auth_scheme v = ok true ∧ t = none) ∨
                      (is_non_bearer_auth_scheme v = ok false ∧ t = some v)))))))) := by
  intro value t
  constructor
  · intro hval
    unfold bearer_token_from_value at hval
    obtain ⟨v, hw, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next htrue =>
      refine ⟨v, hw, Or.inl ⟨by rw [hb, htrue], ?_⟩⟩
      injection hval with ht
      exact ht.symm
    · next hfalse =>
      simp only [Bool.not_eq_true] at hfalse
      refine ⟨v, hw, Or.inr ⟨by rw [hb, hfalse], ?_⟩⟩
      obtain ⟨o, hsp, hval⟩ := bind_ok_inv _ _ _ hval
      split at hval
      · next scheme rest =>
        obtain ⟨br, hbr, hval⟩ := bind_ok_inv _ _ _ hval
        refine Or.inl ⟨scheme, rest, hsp, ?_⟩
        split at hval
        · next hbrtrue =>
          obtain ⟨tok, htk, hval⟩ := bind_ok_inv _ _ _ hval
          obtain ⟨e, he, hval⟩ := bind_ok_inv _ _ _ hval
          refine Or.inl ⟨by rw [hbr, hbrtrue], tok, htk, ?_⟩
          split at hval
          · next hetrue =>
            exact Or.inl ⟨by rw [he, hetrue], by injection hval with ht; exact ht.symm⟩
          · next hefalse =>
            simp only [Bool.not_eq_true] at hefalse
            exact Or.inr ⟨by rw [he, hefalse], by injection hval with ht; exact ht.symm⟩
        · next hbrfalse =>
          simp only [Bool.not_eq_true] at hbrfalse
          refine Or.inr ⟨by rw [hbr, hbrfalse], by injection hval with ht; exact ht.symm⟩
      · next =>
        obtain ⟨br, hbr, hval⟩ := bind_ok_inv _ _ _ hval
        refine Or.inr ⟨hsp, ?_⟩
        split at hval
        · next hbrtrue =>
          exact Or.inl ⟨by rw [hbr, hbrtrue], by injection hval with ht; exact ht.symm⟩
        · next hbrfalse =>
          simp only [Bool.not_eq_true] at hbrfalse
          refine Or.inr ⟨by rw [hbr, hbrfalse], ?_⟩
          obtain ⟨nb, hnb, hval⟩ := bind_ok_inv _ _ _ hval
          split at hval
          · next hnbtrue =>
            exact Or.inl ⟨by rw [hnb, hnbtrue], by injection hval with ht; exact ht.symm⟩
          · next hnbfalse =>
            simp only [Bool.not_eq_true] at hnbfalse
            exact Or.inr ⟨by rw [hnb, hnbfalse], by injection hval with ht; exact ht.symm⟩
  · rintro ⟨v, hw, hbranch⟩
    rcases hbranch with ⟨he, ht⟩ | ⟨he, hsplit⟩
    · unfold bearer_token_from_value
      simp [hw, he, ht]
    · rcases hsplit with ⟨scheme, rest, hsp, hEF⟩ | ⟨hsp, hGH⟩
      · rcases hEF with ⟨hbr, tok, htk, hIJ⟩ | ⟨hbr, ht⟩
        · rcases hIJ with ⟨he2, ht⟩ | ⟨he2, ht⟩
          · unfold bearer_token_from_value
            simp [hw, he, hsp, hbr, htk, he2, ht]
          · unfold bearer_token_from_value
            simp [hw, he, hsp, hbr, htk, he2, ht]
        · unfold bearer_token_from_value
          simp [hw, he, hsp, hbr, ht]
      · rcases hGH with ⟨hbr, ht⟩ | ⟨hbr, hH⟩
        · unfold bearer_token_from_value
          simp [hw, he, hsp, hbr, ht]
        · rcases hH with ⟨hnb, ht⟩ | ⟨hnb, ht⟩
          · unfold bearer_token_from_value
            simp [hw, he, hsp, hbr, hnb, ht]
          · unfold bearer_token_from_value
            simp [hw, he, hsp, hbr, hnb, ht]

/- RFC-0215 P2.1 1/6 (átomo `catalog:ascii_eq_ignore_case`, entrada
`is_bearer_scheme`): o gate do scheme Bearer decide exatamente na
comparação case-fold ASCII contra o token `bearer` (RFC 9110) — o
corpo é a chamada única, citada não reaberta. O mutante AS-IS
(`is_bearer_scheme_as_is`) só casa os dois literais; planta
`bearer_case_insensitive` recusa. -/
theorem is_bearer_scheme_fate_iff :
    ∀ (scheme : Str) (v : Bool),
      (is_bearer_scheme scheme = ok v) ↔
        core.str.Str.eq_ignore_ascii_case scheme (toStr "bearer") = ok v := by
  intro scheme v
  unfold is_bearer_scheme
  rfl

/- RFC-0215 P2.1 2/6 (átomo `catalog:ascii_eq_ignore_case`, entrada
`is_non_bearer_auth_scheme`): o gate dos outros auth-schemes decide
exatamente na cadeia de comparações case-fold (basic → digest →
negotiate → ntlm) — verdadeiro no primeiro que casa, senão o
veredito da última comparação; cada ramo carrega a igualdade
habilitante. O mutante AS-IS recusa tudo (scheme-blind, F150/F151);
planta `non_bearer_scheme_gate` recusa. -/
theorem is_non_bearer_auth_scheme_fate_iff :
    ∀ (scheme : Str) (v : Bool),
      (is_non_bearer_auth_scheme scheme = ok v) ↔
        ∃ b0, core.str.Str.eq_ignore_ascii_case scheme (toStr "basic") = ok b0 ∧
          ((b0 = true ∧ v = true) ∨
            (b0 = false ∧
              ∃ b1, core.str.Str.eq_ignore_ascii_case scheme (toStr "digest") = ok b1 ∧
                ((b1 = true ∧ v = true) ∨
                  (b1 = false ∧
                    ∃ b2, core.str.Str.eq_ignore_ascii_case scheme (toStr "negotiate") = ok b2 ∧
                      ((b2 = true ∧ v = true) ∨
                        (b2 = false ∧
                          core.str.Str.eq_ignore_ascii_case scheme (toStr "ntlm") = ok v)))))) := by
  intro scheme v
  constructor
  · intro hval
    unfold is_non_bearer_auth_scheme at hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
        exact ⟨b, hb, Or.inl ⟨hbt, (Result.ok.inj hval).symm⟩⟩
    · next hbf =>
        simp only [Bool.not_eq_true] at hbf
        obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
        split at hval
        · next hb1t =>
            exact ⟨b, hb, Or.inr ⟨hbf, b1, hb1,
              Or.inl ⟨hb1t, (Result.ok.inj hval).symm⟩⟩⟩
        · next hb1f =>
            simp only [Bool.not_eq_true] at hb1f
            obtain ⟨b2, hb2, hval⟩ := bind_ok_inv _ _ _ hval
            split at hval
            · next hb2t =>
                exact ⟨b, hb, Or.inr ⟨hbf, b1, hb1, Or.inr ⟨hb1f, b2, hb2,
                  Or.inl ⟨hb2t, (Result.ok.inj hval).symm⟩⟩⟩⟩
            · next hb2f =>
                simp only [Bool.not_eq_true] at hb2f
                exact ⟨b, hb, Or.inr ⟨hbf, b1, hb1, Or.inr ⟨hb1f, b2, hb2,
                  Or.inr ⟨hb2f, hval⟩⟩⟩⟩
  · rintro ⟨b, hb, ⟨hbt, hv⟩ | ⟨hbf, b1, hb1,
      ⟨hb1t, hv⟩ | ⟨hb1f, b2, hb2, ⟨hb2t, hv⟩ | ⟨hb2f, hnt⟩⟩⟩⟩
    · unfold is_non_bearer_auth_scheme
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbt, if_pos rfl, hv]
    · unfold is_non_bearer_auth_scheme
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbf, if_neg (by simp)]
      rw [hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hb1t, if_pos rfl, hv]
    · unfold is_non_bearer_auth_scheme
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbf, if_neg (by simp)]
      rw [hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hb1f, if_neg (by simp)]
      rw [hb2]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hb2t, if_pos rfl, hv]
    · unfold is_non_bearer_auth_scheme
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbf, if_neg (by simp)]
      rw [hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hb1f, if_neg (by simp)]
      rw [hb2]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hb2f, if_neg (by simp)]
      exact hnt

/- RFC-0215 P2.1 3/6 (átomo `catalog:ascii_upper`, entrada
`normalize_http_method`): o token do método HTTP normaliza
exatamente na dobra ASCII-upcase do extrato (RFC 9110 compara o
método em caixa alta) — o corpo é a chamada única, citada não
reaberta. O mutante AS-IS devolve o token cru (`put` ≠ `PUT`);
planta `kv_http_method_case_insensitive` recusa. -/
theorem normalize_http_method_fate_iff :
    ∀ (m : Str) (r : String),
      (normalize_http_method m = ok r) ↔
        alloc.str.Str.to_ascii_uppercase m = ok r := by
  intro m r
  unfold normalize_http_method
  rfl

/- RFC-0215 P2.1 4/6 (átomo `catalog:ascii_lower`, entrada
`ascii_lower`): a dobra de byte para caixa baixa decide exatamente
no teste `is_ascii_uppercase` — byte maiúsculo vira o veredito da
dobra `to_ascii_lowercase`, qualquer outro é ele mesmo; cada ramo
carrega a igualdade habilitante. O mutante AS-IS não dobra (`BEARER`
nunca casa `bearer`, F85); planta `ascii_fold_discriminates_as_is`
recusa. -/
theorem ascii_lower_fate_iff :
    ∀ (b c : U8),
      (ascii_lower b = ok c) ↔
        ∃ b1, core.num.U8.is_ascii_uppercase b = ok b1 ∧
          ((b1 = true ∧ core.num.U8.to_ascii_lowercase b = ok c) ∨
            (b1 = false ∧ c = b)) := by
  intro b c
  constructor
  · intro hval
    unfold ascii_lower at hval
    obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt => exact ⟨b1, hb1, Or.inl ⟨hbt, hval⟩⟩
    · next hbf =>
        simp only [Bool.not_eq_true] at hbf
        exact ⟨b1, hb1, Or.inr ⟨hbf, (Result.ok.inj hval).symm⟩⟩
  · rintro ⟨b1, hb1, ⟨hbt, hfold⟩ | ⟨hbf, hcb⟩⟩
    · unfold ascii_lower
      rw [hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbt, if_pos rfl]
      exact hfold
    · unfold ascii_lower
      rw [hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbf, if_neg (by simp), hcb]

/- RFC-0215 P2.1 5/6 (átomo `catalog:ascii_upper`, entrada
`ascii_upper`): a dobra de byte para caixa alta decide exatamente
no teste `is_ascii_lowercase` — byte minúsculo vira o veredito da
dobra `to_ascii_uppercase`, qualquer outro é ele mesmo; cada ramo
carrega a igualdade habilitante. O mutante AS-IS não dobra (`put`
nunca casa `PUT`, F79); planta `ascii_fold_discriminates_as_is`
recusa. -/
theorem ascii_upper_fate_iff :
    ∀ (b c : U8),
      (ascii_upper b = ok c) ↔
        ∃ b1, core.num.U8.is_ascii_lowercase b = ok b1 ∧
          ((b1 = true ∧ core.num.U8.to_ascii_uppercase b = ok c) ∨
            (b1 = false ∧ c = b)) := by
  intro b c
  constructor
  · intro hval
    unfold ascii_upper at hval
    obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt => exact ⟨b1, hb1, Or.inl ⟨hbt, hval⟩⟩
    · next hbf =>
        simp only [Bool.not_eq_true] at hbf
        exact ⟨b1, hb1, Or.inr ⟨hbf, (Result.ok.inj hval).symm⟩⟩
  · rintro ⟨b1, hb1, ⟨hbt, hfold⟩ | ⟨hbf, hcb⟩⟩
    · unfold ascii_upper
      rw [hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbt, if_pos rfl]
      exact hfold
    · unfold ascii_upper
      rw [hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hbf, if_neg (by simp), hcb]

/-! ## RFC-0215 P2.1 6/6 — `authorization_matches` (loop real do extrato)

O fate do scan inteiro é a CADEIA dos passos citados: cada header
varrido entra com suas chamadas ok (index, `as_ref` ×2, eqig e o
ramo — igualdades habilitantes, corpos não reabertos); o fim é o hit
Bearer, ou (no fim do vetor) o veredito — um reject trava `saw`,
senão o fallback `Option.eq` sobre o primeiro X-Pedra-Token. Ramos
fail do corpo quebram os dois lados (a cadeia exige ok). -/

/-- Veredito final do fallback: a chamada `Option.eq` do corpo,
citada não reaberta, sobre o x acumulado (nenhum X-Pedra-Token ou o
primeiro). -/
private def FinalVerdict (expected : Str) (x : Option Str) (v : Bool) : Prop :=
  (x = none ∧
      core.option.Option.Insts.CoreCmpPartialEqOption.eq
        Str.Insts.CoreCmpPartialEqStr none (some expected) = ok v) ∨
    (∃ w, x = some w ∧
      core.option.Option.Insts.CoreCmpPartialEqOption.eq
        Str.Insts.CoreCmpPartialEqStr (some w) (some expected) = ok v)

/-- Um passo `cont` do corpo no header j: a cadeia de chamados ok
(index, `as_ref` ×2, eqig e o ramo), com o estado (saw, x) que sai. -/
private def ScanStep {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str)
    (saw : Bool) (x : Option Str) (j : Nat)
    (hj : j < headers.val.length) (saw' : Bool) (x' : Option Str) : Prop :=
  ∃ (k1 v1 : Str),
    iK.as_ref (headers.val[j]).1 = ok k1 ∧
      iV.as_ref (headers.val[j]).2 = ok v1 ∧
      ((core.str.Str.eq_ignore_ascii_case k1 (toStr "authorization") = ok true ∧
          ((bearer_token_from_value v1 = ok none ∧ saw' = saw ∧ x' = x) ∨
            (∃ t, bearer_token_from_value v1 = ok (some t) ∧
              Str.Insts.CoreCmpPartialEqStr.eq t expected = ok false ∧
              saw' = true ∧ x' = x))) ∨
        (core.str.Str.eq_ignore_ascii_case k1 (toStr "authorization") = ok false ∧
          ((core.str.Str.eq_ignore_ascii_case k1 (toStr "x-pedra-token") = ok false ∧
              saw' = saw ∧ x' = x) ∨
            (core.str.Str.eq_ignore_ascii_case k1 (toStr "x-pedra-token") = ok true ∧
              saw' = saw ∧
              (x = none ∧ x' = some v1 ∨ ∃ w, x = some w ∧ x' = some w)))))

/-- O passo terminal Bearer: o header j (Authorization) entrega um
token que casa o esperado. -/
private def HitStep {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str) (j : Nat)
    (hj : j < headers.val.length) : Prop :=
  ∃ (k1 v1 t : Str),
    iK.as_ref (headers.val[j]).1 = ok k1 ∧
      iV.as_ref (headers.val[j]).2 = ok v1 ∧
      core.str.Str.eq_ignore_ascii_case k1 (toStr "authorization") = ok true ∧
        bearer_token_from_value v1 = ok (some t) ∧
        Str.Insts.CoreCmpPartialEqStr.eq t expected = ok true

/-- O fate do scan como cadeia: combustível = headers restantes; cada
passo `cont` consume um header (com seu `i+1` ok) e o fim é o hit ou
o veredito final. -/
private def LoopFate {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str) :
    Nat → Bool → Option Str → Usize → Bool → Prop
  | 0, saw, x, i, v =>
      i.val = headers.val.length ∧
        ((saw = true ∧ v = false) ∨ (saw = false ∧ FinalVerdict expected x v))
  | fuel + 1, saw, x, i, v =>
      ((∃ (hj : i.val < headers.val.length),
            HitStep iK iV headers expected i.val hj) ∧
          v = true) ∨
        (i.val = headers.val.length ∧
          ((saw = true ∧ v = false) ∨ (saw = false ∧ FinalVerdict expected x v))) ∨
        (∃ (hj : i.val < headers.val.length) (saw' : Bool) (x' : Option Str) (i' : Usize),
            ScanStep iK iV headers expected saw x i.val hj saw' x' ∧
              (i + 1#usize) = ok i' ∧
                LoopFate iK iV headers expected fuel saw' x' i' v)

/-- O `+1#usize` do corpo vale exatamente `i.val + 1` em Nat. -/
private theorem usize_succ_val (i i1 : Usize) (h : (i + 1#usize) = ok i1) :
    (↑i1 : Nat) = (↑i : Nat) + 1 := by
  have he := UScalar.add_equiv i 1#usize
  rw [h] at he
  dsimp only at he
  exact he.2.1

/-- Análise forward de um passo cont: a igualdade do corpo produz o
ScanStep e as duas equações do sucessor. -/
private theorem scan_step_of_body {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str)
    (saw : Bool) (x : Option Str) (i i' : Usize)
    (saw' : Bool) (x' : Option Str)
    (hi : i.val < headers.val.length)
    (h : authorization_matches_loop.body iK iV headers expected saw x i
          = ok (ControlFlow.cont (saw', x', i'))) :
    ScanStep iK iV headers expected saw x i.val hi saw' x' ∧
      i'.val = i.val + 1 ∧ (i + 1#usize) = ok i' := by
  unfold authorization_matches_loop.body at h
  dsimp +zeta only at h
  split at h
  · rename_i hltU
    obtain ⟨kv, hkv, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨kv0, hkveq, hkvv0⟩ := (Aeneas.Std.WP.spec_equiv_exists _ _).mp
      (Slice.index_usize_spec headers i hi)
    have hkvv : kv = headers.val[i.val] :=
      (Result.ok.inj (hkv.symm.trans hkveq)).trans hkvv0
    obtain ⟨k, v⟩ := kv
    obtain ⟨k1, hk1, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨v1, hv1, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨is_auth, hauth, h⟩ := bind_ok_inv _ _ _ h
    have hk1' : iK.as_ref (headers.val[i.val]).1 = ok k1 := by
      rw [← hkvv]; exact hk1
    have hv1' : iV.as_ref (headers.val[i.val]).2 = ok v1 := by
      rw [← hkvv]; exact hv1
    cases is_auth with
    | true =>
      obtain ⟨tok, htok, h⟩ := bind_ok_inv _ _ _ h
      cases tok with
      | none =>
        obtain ⟨i1, hi1, h⟩ := bind_ok_inv _ _ _ h
        have hinj := Result.ok.inj h
        simp only [ControlFlow.cont.injEq, Prod.mk.injEq] at hinj
        obtain ⟨hs, hx, hi1i'⟩ := hinj
        rw [hi1i'] at hi1
        subst hs; subst hx
        exact ⟨⟨k1, v1, hk1', hv1',
          Or.inl ⟨hauth, Or.inl ⟨htok, rfl, rfl⟩⟩⟩,
          usize_succ_val i i' hi1, hi1⟩
      | some t =>
        obtain ⟨eq, heq, h⟩ := bind_ok_inv _ _ _ h
        cases eq with
        | true =>
          dsimp only at h
          injection h with h2
          contradiction
        | false =>
          obtain ⟨i1, hi1, h⟩ := bind_ok_inv _ _ _ h
          have hinj := Result.ok.inj h
          simp only [ControlFlow.cont.injEq, Prod.mk.injEq] at hinj
          obtain ⟨hs, hx, hi1i'⟩ := hinj
          rw [hi1i'] at hi1
          subst hs; subst hx
          exact ⟨⟨k1, v1, hk1', hv1',
            Or.inl ⟨hauth, Or.inr ⟨t, htok, heq, rfl, rfl⟩⟩⟩,
            usize_succ_val i i' hi1, hi1⟩
    | false =>
      obtain ⟨is_xp, hxp, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨x2, hx2, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨i1, hi1, h⟩ := bind_ok_inv _ _ _ h
      have hinj := Result.ok.inj h
      simp only [ControlFlow.cont.injEq, Prod.mk.injEq] at hinj
      obtain ⟨hs, hx2i', hi1i'⟩ := hinj
      rw [hi1i'] at hi1
      subst hs
      cases is_xp with
      | true =>
        cases x with
        | none =>
          dsimp only at hx2
          have hx2v : x2 = some v1 := Result.ok.inj hx2.symm
          subst hx2v; subst hx2i'
          exact ⟨⟨k1, v1, hk1', hv1',
            Or.inr ⟨hauth, Or.inr ⟨hxp, rfl, Or.inl ⟨rfl, rfl⟩⟩⟩⟩,
            usize_succ_val i i' hi1, hi1⟩
        | some w =>
          dsimp only at hx2
          have hx2x : x2 = some w := Result.ok.inj hx2.symm
          subst hx2x; subst hx2i'
          exact ⟨⟨k1, v1, hk1', hv1',
            Or.inr ⟨hauth, Or.inr ⟨hxp, rfl, Or.inr ⟨w, rfl, rfl⟩⟩⟩⟩,
            usize_succ_val i i' hi1, hi1⟩
      | false =>
        dsimp only at hx2
        have hx2x : x2 = x := Result.ok.inj hx2.symm
        subst hx2x; subst hx2i'
        exact ⟨⟨k1, v1, hk1', hv1',
          Or.inr ⟨hauth, Or.inl ⟨hxp, rfl, rfl⟩⟩⟩,
          usize_succ_val i i' hi1, hi1⟩
  · rename_i hgeU
    have hlt' : i < Slice.len headers := by
      refine (UScalar.lt_equiv i (Slice.len headers)).mpr ?_
      rw [Aeneas.Std.Slice.len_val]
      exact hi
    exact absurd hlt' hgeU

/-- Reassembly: o ScanStep reconstrói o valor do corpo. -/
private theorem body_of_scan_step {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str)
    (saw : Bool) (x : Option Str) (i i' : Usize)
    (saw' : Bool) (x' : Option Str)
    (hi : i.val < headers.val.length)
    (hStep : ScanStep iK iV headers expected saw x i.val hi saw' x')
    (hAdd : (i + 1#usize) = ok i') :
    authorization_matches_loop.body iK iV headers expected saw x i
      = ok (ControlFlow.cont (saw', x', i')) := by
  obtain ⟨k1, v1, hk1, hv1, hbranch⟩ := hStep
  obtain ⟨kv, hkveq, hkvv⟩ := (Aeneas.Std.WP.spec_equiv_exists _ _).mp
    (Slice.index_usize_spec headers i hi)
  obtain ⟨k, v⟩ := kv
  have hk1k : iK.as_ref k = ok k1 := by rw [← hkvv] at hk1; exact hk1
  have hv1v : iV.as_ref v = ok v1 := by rw [← hkvv] at hv1; exact hv1
  unfold authorization_matches_loop.body
  dsimp +zeta only
  split
  · rw [hkveq]
    simp only [Aeneas.Std.bind_tc_ok]
    conv => lhs; whnf
    rw [hk1k]
    split
    · next r a heq =>
      injection heq with hka
      subst hka
      dsimp only
      rw [hv1v]
      simp only [Aeneas.Std.bind_tc_ok]
      rcases hbranch with ⟨hauth, ⟨htok, hsaw, hx⟩ | ⟨t, htok, heqf, hsaw, hx⟩⟩ |
        ⟨hauth, ⟨hxp, hsaw, hx⟩ | ⟨hxp, hsaw, ⟨hx1n, hx1⟩ | ⟨w, hx1w, hx1⟩⟩⟩
      · simp [hauth, htok, hAdd, hsaw, hx]
      · simp [hauth, htok, heqf, hAdd, hsaw, hx]
      · simp [hauth, hxp, hAdd, hsaw, hx]
      · cases x with
        | none => simp [hauth, hxp, hAdd, hsaw, hx1]
        | some w' => simp at hx1n
      · cases x with
        | none => simp at hx1w
        | some w' =>
          injection hx1w with hw
          subst hw
          simp [hauth, hxp, hAdd, hsaw, hx1]
    · next r1 e heq => contradiction
    · next r2 heq => contradiction
  · rename_i hgeU
    have hlt' : i < Slice.len headers := by
      refine (UScalar.lt_equiv i (Slice.len headers)).mpr ?_
      rw [Aeneas.Std.Slice.len_val]
      exact hi
    exact absurd hlt' hgeU

/-- Análise forward do hit: done só sai com r = true no passo Bearer. -/
private theorem hit_of_body {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str)
    (saw : Bool) (x : Option Str) (i : Usize) (r : Bool)
    (hi : i.val < headers.val.length)
    (h : authorization_matches_loop.body iK iV headers expected saw x i
          = ok (ControlFlow.done r)) :
    r = true ∧ HitStep iK iV headers expected i.val hi := by
  have hlt : i < Slice.len headers := by
    refine (UScalar.lt_equiv i (Slice.len headers)).mpr ?_
    rw [Aeneas.Std.Slice.len_val]
    exact hi
  unfold authorization_matches_loop.body at h
  dsimp +zeta only at h
  split at h
  · rename_i hltU
    obtain ⟨kv, hkv, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨kv0, hkveq, hkvv0⟩ := (Aeneas.Std.WP.spec_equiv_exists _ _).mp
      (Slice.index_usize_spec headers i hi)
    have hkvv : kv = headers.val[i.val] :=
      (Result.ok.inj (hkv.symm.trans hkveq)).trans hkvv0
    obtain ⟨k, v⟩ := kv
    obtain ⟨k1, hk1, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨v1, hv1, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨is_auth, hauth, h⟩ := bind_ok_inv _ _ _ h
    have hk1' : iK.as_ref (headers.val[i.val]).1 = ok k1 := by
      rw [← hkvv]; exact hk1
    have hv1' : iV.as_ref (headers.val[i.val]).2 = ok v1 := by
      rw [← hkvv]; exact hv1
    cases is_auth with
    | true =>
      obtain ⟨tok, htok, h⟩ := bind_ok_inv _ _ _ h
      cases tok with
      | some t =>
        obtain ⟨eq, heq, h⟩ := bind_ok_inv _ _ _ h
        cases eq with
        | true =>
          dsimp only at h
          exact ⟨(ControlFlow.done.inj (Result.ok.inj h)).symm,
            ⟨k1, v1, t, hk1', hv1', hauth, htok, heq⟩⟩
        | false =>
          obtain ⟨i1, -, hb⟩ := bind_ok_inv _ _ _ h
          injection hb with hb2
          contradiction
      | none =>
        obtain ⟨i1, -, hb⟩ := bind_ok_inv _ _ _ h
        injection hb with hb2
        contradiction
    | false =>
      obtain ⟨is_xp, -, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨x2, -, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨i1, -, hb⟩ := bind_ok_inv _ _ _ h
      injection hb with hb2
      contradiction
  · rename_i hgeU
    exact absurd hlt hgeU

/-- Reassembly do hit: o HitStep reconstrói done true. -/
private theorem body_of_hit {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str)
    (saw : Bool) (x : Option Str) (i : Usize)
    (hi : i.val < headers.val.length)
    (hhit : HitStep iK iV headers expected i.val hi) :
    authorization_matches_loop.body iK iV headers expected saw x i
      = ok (ControlFlow.done true) := by
  obtain ⟨k1, v1, t, hk1, hv1, hauth, htok, heq⟩ := hhit
  obtain ⟨kv, hkveq, hkvv⟩ := (Aeneas.Std.WP.spec_equiv_exists _ _).mp
    (Slice.index_usize_spec headers i hi)
  obtain ⟨k, v⟩ := kv
  have hk1k : iK.as_ref k = ok k1 := by rw [← hkvv] at hk1; exact hk1
  have hv1v : iV.as_ref v = ok v1 := by rw [← hkvv] at hv1; exact hv1
  unfold authorization_matches_loop.body
  dsimp +zeta only
  split
  · rw [hkveq]
    simp only [Aeneas.Std.bind_tc_ok]
    conv => lhs; whnf
    rw [hk1k]
    split
    · next r a hok =>
      injection hok with hka
      subst hka
      dsimp only
      rw [hv1v]
      simp only [Aeneas.Std.bind_tc_ok]
      simp [hauth, htok, heq]
    · next r1 e heq => contradiction
    · next r2 heq => contradiction
  · rename_i hgeU
    have hlt' : i < Slice.len headers := by
      refine (UScalar.lt_equiv i (Slice.len headers)).mpr ?_
      rw [Aeneas.Std.Slice.len_val]
      exact hi
    exact absurd hlt' hgeU

/-- No fim (i = len) o corpo nunca dá cont. -/
private theorem body_no_cont_at_end {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str)
    (saw : Bool) (x : Option Str) (i : Usize) (st : Bool × Option Str × Usize)
    (heq : i.val = headers.val.length)
    (hB : authorization_matches_loop.body iK iV headers expected saw x i
          = ok (ControlFlow.cont st)) : False := by
  have hge : ¬ (i < Slice.len headers) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Slice.len headers)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    have hn : i.val < headers.val.length := hn0
    rw [heq] at hn
    exact absurd hn (Nat.lt_irrefl _)
  unfold authorization_matches_loop.body at hB
  dsimp +zeta only at hB
  rw [if_neg hge] at hB
  cases saw with
  | true =>
    dsimp only at hB
    injection hB with hB2
    contradiction
  | false =>
    dsimp only at hB
    obtain ⟨b, -, hb⟩ := bind_ok_inv _ _ _ hB
    injection hb with hb2
    contradiction

/-- O ramo final (i = len) sem loop: veredito exatamente per saw/x. -/
private theorem body_final_iff {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str)
    (saw : Bool) (x : Option Str) (i : Usize) (v : Bool)
    (heq : i.val = headers.val.length) :
    (authorization_matches_loop.body iK iV headers expected saw x i
        = ok (ControlFlow.done v)) ↔
      ((saw = true ∧ v = false) ∨ (saw = false ∧ FinalVerdict expected x v)) := by
  have hge : ¬ (i < Slice.len headers) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Slice.len headers)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    have hn : i.val < headers.val.length := hn0
    rw [heq] at hn
    exact absurd hn (Nat.lt_irrefl _)
  unfold authorization_matches_loop.body
  dsimp +zeta only
  rw [if_neg hge]
  cases saw with
  | true =>
    dsimp only
    constructor
    · intro h
      exact Or.inl ⟨rfl, (ControlFlow.done.inj (Result.ok.inj h)).symm⟩
    · intro hf
      rcases hf with ⟨-, hv⟩ | ⟨hs, -⟩
      · rw [hv]
      · exact Bool.noConfusion hs
  | false =>
    dsimp only
    constructor
    · intro h
      obtain ⟨b, hb, h'⟩ := bind_ok_inv _ _ _ h
      have hbv : b = v := ControlFlow.done.inj (Result.ok.inj h')
      refine Or.inr ⟨rfl, ?_⟩
      cases x with
      | none => exact Or.inl ⟨rfl, by rw [← hbv]; exact hb⟩
      | some w => exact Or.inr ⟨w, rfl, by rw [← hbv]; exact hb⟩
    · intro hf
      rcases hf with ⟨hs, -⟩ | ⟨-, hfin⟩
      · exact Bool.noConfusion hs
      · rcases hfin with ⟨hx, hopt⟩ | ⟨w, hx, hopt⟩
        · subst hx
          rw [hopt]
          simp only [Aeneas.Std.bind_tc_ok]
        · subst hx
          rw [hopt]
          simp only [Aeneas.Std.bind_tc_ok]

/-- O fate do loop por indução no combustível. -/
private theorem auth_loop_fate {K : Type} {V : Type}
    (iK : core.convert.AsRef K Str) (iV : core.convert.AsRef V Str)
    (headers : Slice (K × V)) (expected : Str) :
    ∀ (fuel : Nat) (saw : Bool) (x : Option Str) (i : Usize),
      i.val ≤ headers.val.length →
      headers.val.length - i.val ≤ fuel →
      ∀ v : Bool,
        (authorization_matches_loop iK iV headers expected saw x i = ok v) ↔
          LoopFate iK iV headers expected fuel saw x i v := by
  intro fuel
  induction fuel with
  | zero =>
    intro saw x i hile hfuel v
    have hlen : i.val = headers.val.length := by omega
    constructor
    · intro h
      refine ⟨hlen, ?_⟩
      unfold authorization_matches_loop at h
      rw [loop.eq_def] at h
      dsimp only at h
      cases hB : authorization_matches_loop.body iK iV headers expected saw x i with
      | ok cf =>
        cases cf with
        | done r =>
          rw [hB] at h
          dsimp only at h
          rw [Result.ok.inj h] at hB
          exact (body_final_iff iK iV headers expected saw x i v hlen).mp hB
        | cont st =>
          exact absurd hB (body_no_cont_at_end iK iV headers expected saw x i st hlen)
      | fail e =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
      | div =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
    · rintro ⟨-, hverdict⟩
      have hbody := (body_final_iff iK iV headers expected saw x i v hlen).mpr hverdict
      unfold authorization_matches_loop
      rw [loop.eq_def]
      dsimp only
      rw [hbody]
  | succ fuel ih =>
    intro saw x i hile hfuel v
    unfold authorization_matches_loop
    rw [loop.eq_def]
    dsimp only
    cases hB : authorization_matches_loop.body iK iV headers expected saw x i with
    | ok cf =>
      cases cf with
      | cont st =>
        obtain ⟨saw', x', i'⟩ := st
        dsimp only
        by_cases hlt : i.val < headers.val.length
        · obtain ⟨hstep, hval, hAdd⟩ :=
            scan_step_of_body iK iV headers expected saw x i i' saw' x' hlt hB
          constructor
          · intro h
            refine Or.inr (Or.inr ⟨hlt, saw', x', i', hstep, hAdd, ?_⟩)
            exact (ih saw' x' i' (by omega) (by omega) v).mp h
          · intro hf
            rcases hf with ⟨⟨hj, hhit⟩, hv⟩ |
              (⟨hlen, hverdict⟩ |
                ⟨hj, saw2, x2, i2, hstep2, hAdd2, hfate⟩)
            · exact absurd (hB.symm.trans (body_of_hit iK iV headers expected saw x i hj hhit))
                (by simp)
            · exact absurd hlt (by omega)
            · have hval2 : i2.val = i.val + 1 := usize_succ_val i i2 hAdd2
              have hbody2 := body_of_scan_step iK iV headers expected saw x i i2 saw2 x2 hj
                hstep2 hAdd2
              have hst : (saw', x', i') = (saw2, x2, i2) := by
                have hc := Result.ok.inj (hB.symm.trans hbody2)
                simpa [ControlFlow.cont.injEq, Prod.mk.injEq] using hc
              rw [hst]
              exact (ih saw2 x2 i2 (by omega) (by omega) v).mpr hfate
        · have hlen : i.val = headers.val.length := by omega
          have hfalse := body_no_cont_at_end iK iV headers expected saw x i
            (saw', x', i') hlen hB
          constructor
          · intro h
            exact hfalse.elim
          · intro hf
            exact hfalse.elim
      | done r =>
        dsimp only
        by_cases hlt : i.val < headers.val.length
        · obtain ⟨hr, hhit⟩ := hit_of_body iK iV headers expected saw x i r hlt hB
          constructor
          · intro h
            exact Or.inl ⟨⟨hlt, hhit⟩, (Result.ok.inj h).symm.trans hr⟩
          · intro hf
            rcases hf with ⟨⟨hj, hhit⟩, hv⟩ |
              (⟨hlen, hverdict⟩ |
                ⟨hj, saw2, x2, i2, hstep2, hAdd2, -⟩)
            · rw [hr, hv]
            · exact absurd hlt (by omega)
            · exact absurd (hB.symm.trans
                (body_of_scan_step iK iV headers expected saw x i i2 saw2 x2 hj hstep2 hAdd2))
                (by simp)
        · have hlen : i.val = headers.val.length := by omega
          have hfin := (body_final_iff iK iV headers expected saw x i r hlen).mp hB
          constructor
          · intro h
            have hrv : r = v := Result.ok.inj h
            refine Or.inr (Or.inl ⟨hlen, ?_⟩)
            rcases hfin with ⟨hs, hv⟩ | ⟨hs, hf⟩
            · exact Or.inl ⟨hs, hrv ▸ hv⟩
            · exact Or.inr ⟨hs, hrv ▸ hf⟩
          · intro hf
            rcases hf with ⟨⟨hj, hhit⟩, -⟩ |
              (⟨hlen2, hverdict⟩ |
                ⟨hj, saw2, x2, i2, hstep2, hAdd2, -⟩)
            · exact absurd hlt (by omega)
            · have hbody := (body_final_iff iK iV headers expected saw x i v hlen).mpr hverdict
              have hr2 : r = v := by
                have hu := hB.symm.trans hbody
                injection hu with hu2
                injection hu2 with hrv
              rw [hr2]
            · exact absurd (hB.symm.trans
                (body_of_scan_step iK iV headers expected saw x i i2 saw2 x2 hj hstep2 hAdd2))
                (by simp)
    | fail e =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · intro hf
        rcases hf with ⟨⟨hj, hhit⟩, -⟩ |
          (⟨hlen, hverdict⟩ |
            ⟨hj, saw2, x2, i2, hstep2, hAdd2, -⟩)
        · exact absurd (hB.symm.trans (body_of_hit iK iV headers expected saw x i hj hhit))
            (by simp)
        · exact absurd (hB.symm.trans
            ((body_final_iff iK iV headers expected saw x i v hlen).mpr hverdict)) (by simp)
        · exact absurd (hB.symm.trans
            (body_of_scan_step iK iV headers expected saw x i i2 saw2 x2 hj hstep2 hAdd2))
            (by simp)
    | div =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · intro hf
        rcases hf with ⟨⟨hj, hhit⟩, -⟩ |
          (⟨hlen, hverdict⟩ |
            ⟨hj, saw2, x2, i2, hstep2, hAdd2, -⟩)
        · exact absurd (hB.symm.trans (body_of_hit iK iV headers expected saw x i hj hhit))
            (by simp)
        · exact absurd (hB.symm.trans
            ((body_final_iff iK iV headers expected saw x i v hlen).mpr hverdict)) (by simp)
        · exact absurd (hB.symm.trans
            (body_of_scan_step iK iV headers expected saw x i i2 saw2 x2 hj hstep2 hAdd2))
            (by simp)

/-- RFC-0215 P2.1 6/6 (átomo `catalog:x_pedra_is_fallback_only`,
entrada `authorization_matches`): o veredito do cabeçalho é exatamente
a cadeia citada do scan — hit Bearer no primeiro Authorization com
token que casa; um reject trava o fallback; senão o primeiro
X-Pedra-Token decide via a chamada `Option.eq` (fallback only, F149).
O mutante AS-IS deixa um Bearer dummy trancar o token válido;
planta `authorization_matches_on_live_http_is_not_ok` recusa. -/
theorem authorization_matches_fate_iff :
    ∀ {K : Type} {V : Type} (iK : core.convert.AsRef K Str)
      (iV : core.convert.AsRef V Str) (headers : Slice (K × V))
      (expected : Str) (v : Bool),
      (authorization_matches iK iV headers expected = ok v) ↔
        LoopFate iK iV headers expected headers.val.length false none 0#usize v := by
  intro K V iK iV headers expected v
  unfold authorization_matches
  exact auth_loop_fate iK iV headers expected headers.val.length false none 0#usize
    (Nat.zero_le _) (by omega) v
