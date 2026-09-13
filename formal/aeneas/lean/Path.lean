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

/-! ### RFC-0216 P2.1 — path ×8 átomo -/

private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- RFC-0216 P2.1 1/8 (átomo `catalog:strip_authority_for_routing`):
  o roteador da forma-authority repassa exatamente a bandeira de
  forma-authority; o AS-IS nunca strips (dente já provado acima). -/
theorem strip_authority_for_routing_fate_iff :
    ∀ (b : Bool) (r : Bool),
      (strip_authority_for_routing b = ok r) ↔ r = b := by
  intro b r
  constructor
  · intro hval
    unfold strip_authority_for_routing at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold strip_authority_for_routing
    rw [hr]

/-- RFC-0216 P2.1 2/8 (átomo `catalog:strip_uri_fragment`): o
  fragmento é descartado exatamente pelo split no `#` — sem `#` a
  target volta inteira, com `#` fica o prefixo. -/
theorem strip_uri_fragment_fate_iff :
    ∀ (t : Str) (r : Str),
      (strip_uri_fragment t = ok r) ↔
        ((core.str.Str.split_once t '#' = ok none ∧ r = t) ∨
          (∃ (a : Str) (snd : Str),
              core.str.Str.split_once t '#' = ok (some (a, snd)) ∧ r = a)) := by
  intro t r
  constructor
  · intro hval
    unfold strip_uri_fragment at hval
    obtain ⟨o, ho, hval⟩ := bind_ok_inv _ _ _ hval
    cases o with
    | none =>
      dsimp only at hval
      exact Or.inl ⟨ho, (Result.ok.inj hval).symm⟩
    | some pair =>
      obtain ⟨a, snd⟩ := pair
      dsimp only at hval
      exact Or.inr ⟨a, snd, ho, (Result.ok.inj hval).symm⟩
  · rintro (⟨ho, rfl⟩ | ⟨a, snd, ho, rfl⟩)
    · unfold strip_uri_fragment
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
    · unfold strip_uri_fragment
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]

/-- RFC-0216 P2.1 3/8 (átomo `catalog:path_after_authority`): o path
  depois da autoridade é exatamente o primeiro `/` em diante — sem
  `/` a resposta é a raiz `/`, com `/` é o slice index a partir
  dele. -/
theorem path_after_authority_fate_iff :
    ∀ (rest : Str) (r : Str),
      (path_after_authority rest = ok r) ↔
        ((core.str.Str.find rest '/' = ok none ∧
            r = toStr "/" path_after_authority._proof_1) ∨
          (∃ (i : Usize),
              core.str.Str.find rest '/' = ok (some i) ∧
                Str.Insts.CoreOpsIndexIndex.index
                  core.ops.range.RangeFromUsize.Insts.CoreSliceIndexSliceIndexStrStr
                  rest { start := i } = ok r)) := by
  intro rest r
  constructor
  · intro hval
    unfold path_after_authority at hval
    obtain ⟨o, ho, hval⟩ := bind_ok_inv _ _ _ hval
    cases o with
    | none =>
      dsimp only at hval
      exact Or.inl ⟨ho, (Result.ok.inj hval).symm⟩
    | some i =>
      dsimp only at hval
      exact Or.inr ⟨i, ho, hval⟩
  · rintro (⟨ho, rfl⟩ | ⟨i, ho, hindex⟩)
    · unfold path_after_authority
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
    · unfold path_after_authority
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
      exact hindex

/-- RFC-0216 P2.1 4/8 (átomo `catalog:strip_http_authority`): a
  autoridade HTTP é descartada exatamente pelo rest extraído — sem
  `//` prefixo nada a fazer (none), com `//` o path é o
  path_after_authority do rest. -/
theorem strip_http_authority_fate_iff :
    ∀ (t : Str) (r : Option Str),
      (strip_http_authority t = ok r) ↔
        ((strip_http_authority_rest t = ok none ∧ r = none) ∨
          (∃ (rest : Str) (p : Str),
              strip_http_authority_rest t = ok (some rest) ∧
                path_after_authority rest = ok p ∧ r = some p)) := by
  intro t r
  constructor
  · intro hval
    unfold strip_http_authority at hval
    obtain ⟨o, ho, hval⟩ := bind_ok_inv _ _ _ hval
    cases o with
    | none =>
      dsimp only at hval
      exact Or.inl ⟨ho, (Result.ok.inj hval).symm⟩
    | some rest =>
      dsimp only at hval
      obtain ⟨p, hp, hval⟩ := bind_ok_inv _ _ _ hval
      exact Or.inr ⟨rest, p, ho, hp, (Result.ok.inj hval).symm⟩
  · rintro (⟨ho, rfl⟩ | ⟨rest, p, ho, hp, rfl⟩)
    · unfold strip_http_authority
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
    · unfold strip_http_authority
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hp]
      simp only [Aeneas.Std.bind_tc_ok]

/-- RFC-0216 P2.1 5/8 (átomo `catalog:host_authority_mismatch`): o
  Host nunca diverge do authority sem ser detectado — hosts
  diferentes (case-insensitive) ⇒ true; hosts iguais ⇒ o veredito é
  a negação da equivalência de portas. -/
theorem host_authority_mismatch_fate_iff :
    ∀ (host authority : Str) (r : Bool),
      (host_authority_mismatch host authority = ok r) ↔
        (∃ (h1 : Str) (p1 : Option Str) (h2 : Str) (p2 : Option Str),
            split_host_port host = ok (h1, p1) ∧
              split_host_port authority = ok (h2, p2) ∧
                ((core.str.Str.eq_ignore_ascii_case h1 h2 = ok false ∧
                    r = true) ∨
                  (core.str.Str.eq_ignore_ascii_case h1 h2 = ok true ∧
                    ((ports_equivalent p1 p2 = ok true ∧ r = false) ∨
                      (ports_equivalent p1 p2 = ok false ∧ r = true))))) := by
  intro host authority r
  constructor
  · intro hval
    unfold host_authority_mismatch at hval
    obtain ⟨⟨h1, p1⟩, hp1, hval⟩ := bind_ok_inv _ _ _ hval
    simp only [uncurry] at hval
    obtain ⟨⟨h2, p2⟩, hp2, hval⟩ := bind_ok_inv _ _ _ hval
    simp only [uncurry] at hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      rw [hbt] at hb
      obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
      have hr := Result.ok.inj hval
      cases b1 with
      | true =>
        refine ⟨h1, p1, h2, p2, hp1, hp2, ?_⟩
        right
        refine ⟨hb, ?_⟩
        left
        exact ⟨hb1, hr.symm.trans rfl⟩
      | false =>
        refine ⟨h1, p1, h2, p2, hp1, hp2, ?_⟩
        right
        refine ⟨hb, ?_⟩
        right
        exact ⟨hb1, hr.symm.trans rfl⟩
    · next hbf =>
      have hbf' : b = false := by simp at hbf; exact hbf
      rw [hbf'] at hb
      refine ⟨h1, p1, h2, p2, hp1, hp2, ?_⟩
      left
      exact ⟨hb, (Result.ok.inj hval).symm⟩
  · rintro ⟨h1, p1, h2, p2, hp1, hp2,
      (⟨hb, rfl⟩ | ⟨hb, (⟨hb1, rfl⟩ | ⟨hb1, rfl⟩)⟩)⟩
    · unfold host_authority_mismatch
      rw [hp1, hp2]
      simp only [Aeneas.Std.bind_tc_ok]
      simp only [uncurry]
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold host_authority_mismatch
      rw [hp1, hp2]
      simp only [Aeneas.Std.bind_tc_ok]
      simp only [uncurry]
      rw [hb, hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold host_authority_mismatch
      rw [hp1, hp2]
      simp only [Aeneas.Std.bind_tc_ok]
      simp only [uncurry]
      rw [hb, hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      simp

/-- RFC-0216 P2.1 6/8 (átomo `catalog:origin_path`, entrada
  `origin_form_path`): o path de roteamento é exatamente a cadeia
  citada — strip do fragmento, strip da autoridade HTTP (com fallback
  `//` do rest), e o corte no `?`. -/
theorem origin_form_path_fate_iff :
    ∀ (t : Str) (r : Str),
      (origin_form_path t = ok r) ↔
        (∃ (target1 : Str) (o : Option Str) (p : Str),
            strip_uri_fragment t = ok target1 ∧
              strip_http_authority target1 = ok o ∧
                ((∃ (p' : Str), o = some p' ∧ p = p') ∨
                  (o = none ∧
                    ((∃ (rest : Str),
                        core.str.Str.strip_prefix target1
                            (toStr "//" request_target_authority._proof_1) =
                          ok (some rest) ∧
                          path_after_authority rest = ok p) ∨
                      (core.str.Str.strip_prefix target1
                          (toStr "//" request_target_authority._proof_1) =
                          ok none ∧
                        p = target1)))) ∧
                  ((core.str.Str.split_once p '?' = ok none ∧ r = p) ∨
                    (∃ (a : Str) (snd : Str),
                        core.str.Str.split_once p '?' = ok (some (a, snd)) ∧
                          r = a))) := by
  intro t r
  constructor
  · intro hval
    unfold origin_form_path at hval
    obtain ⟨target1, ht1, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨o, ho, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨p, hpO, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨o2, ho2, hval⟩ := bind_ok_inv _ _ _ hval
    cases o2 with
    | none =>
      have hrp : r = p := (Result.ok.inj hval).symm
      cases o with
      | some p' =>
        simp only at hpO
        have hpp : p = p' := (Result.ok.inj hpO).symm
        refine ⟨target1, some p', p, ht1, ho, ?_, ?_⟩
        · left; exact ⟨p', rfl, hpp⟩
        · left; exact ⟨ho2, hrp⟩
      | none =>
        obtain ⟨o1, ho1, hpO⟩ := bind_ok_inv _ _ _ hpO
        cases o1 with
        | some rest =>
          simp only at hpO
          refine ⟨target1, none, p, ht1, ho, ?_, ?_⟩
          · right; refine ⟨rfl, ?_⟩; left; exact ⟨rest, ho1, hpO⟩
          · left; exact ⟨ho2, hrp⟩
        | none =>
          simp only at hpO
          have hpt : p = target1 := (Result.ok.inj hpO).symm
          refine ⟨target1, none, p, ht1, ho, ?_, ?_⟩
          · right; refine ⟨rfl, ?_⟩; right; exact ⟨ho1, hpt⟩
          · left; exact ⟨ho2, hrp⟩
    | some pair =>
      obtain ⟨a, snd⟩ := pair
      have hra : r = a := (Result.ok.inj hval).symm
      cases o with
      | some p' =>
        simp only at hpO
        have hpp : p = p' := (Result.ok.inj hpO).symm
        refine ⟨target1, some p', p, ht1, ho, ?_, ?_⟩
        · left; exact ⟨p', rfl, hpp⟩
        · right; exact ⟨a, snd, ho2, hra⟩
      | none =>
        obtain ⟨o1, ho1, hpO⟩ := bind_ok_inv _ _ _ hpO
        cases o1 with
        | some rest =>
          simp only at hpO
          refine ⟨target1, none, p, ht1, ho, ?_, ?_⟩
          · right; refine ⟨rfl, ?_⟩; left; exact ⟨rest, ho1, hpO⟩
          · right; exact ⟨a, snd, ho2, hra⟩
        | none =>
          simp only at hpO
          have hpt : p = target1 := (Result.ok.inj hpO).symm
          refine ⟨target1, none, p, ht1, ho, ?_, ?_⟩
          · right; refine ⟨rfl, ?_⟩; right; exact ⟨ho1, hpt⟩
          · right; exact ⟨a, snd, ho2, hra⟩
  · rintro ⟨target1, o, p, ht1, ho,
      (⟨p', rfl, rfl⟩ | ⟨rfl, (⟨rest, hsp, hpA⟩ | ⟨hsp, rfl⟩)⟩),
      (⟨ho2, rfl⟩ | ⟨a, snd, ho2, rfl⟩)⟩
    · unfold origin_form_path
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho2]; simp only [Aeneas.Std.bind_tc_ok]
    · unfold origin_form_path
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho2]; simp only [Aeneas.Std.bind_tc_ok]
    · unfold origin_form_path
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hsp]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpA]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho2]; simp only [Aeneas.Std.bind_tc_ok]
    · unfold origin_form_path
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hsp]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpA]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho2]; simp only [Aeneas.Std.bind_tc_ok]
    · unfold origin_form_path
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hsp]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho2]; simp only [Aeneas.Std.bind_tc_ok]
    · unfold origin_form_path
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hsp]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho2]; simp only [Aeneas.Std.bind_tc_ok]
