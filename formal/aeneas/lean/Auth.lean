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
