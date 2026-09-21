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

/-- AS-IS tooth: never strip authority. -/
theorem strip_authority_for_routing_as_is_tooth :
    strip_authority_for_routing_as_is true = ok false := by
  unfold strip_authority_for_routing_as_is
  rfl

/-- AS-IS tooth: fragment stays in the path. -/
theorem strip_uri_fragment_as_is_id (t) :
    strip_uri_fragment_as_is t = ok t := by
  unfold strip_uri_fragment_as_is
  rfl

/-- AS-IS tooth: Host is never compared. -/
theorem host_authority_mismatch_as_is_tooth (h a) :
    host_authority_mismatch_as_is h a = ok false := by
  unfold host_authority_mismatch_as_is
  rfl

/-! ### RFC-0216 P2.1 — path ×8 atom -/

private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- RFC-0216 P2.1 1/8 (atom `catalog:strip_authority_for_routing`):
  o roteador da forma-authority repassa exatamente a bandeira de
  forma-authority; o AS-IS nunca strips (tooth já provado acima). -/
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

/-- RFC-0216 P2.1 2/8 (atom `catalog:strip_uri_fragment`): o
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

/-- RFC-0216 P2.1 3/8 (atom `catalog:path_after_authority`): o path
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

/-- RFC-0216 P2.1 4/8 (atom `catalog:strip_http_authority`): a
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

/-- RFC-0216 P2.1 5/8 (atom `catalog:host_authority_mismatch`): o
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

/-- RFC-0216 P2.1 6/8 (atom `catalog:origin_path`, entrada
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

/-- RFC-0216 P2.1 7/8 (atom `catalog:split_host_port`): o par
  host/porta é exatamente a cadeia citada — o `@` descarta o usuário,
  `[...]` marca um host IPv6 literal com a porta depois de `:`, e
  fora disso o veredito é o `split_host_port_colon`. -/
theorem split_host_port_fate_iff :
    ∀ (raw : Str) (h : Str) (p : Option Str),
      (split_host_port raw = ok (h, p)) ↔
        (∃ (s : Str),
            ((core.str.Str.rsplit_once raw '@' = ok none ∧ s = raw) ∨
              (∃ (user : Str) (s2 : Str),
                  core.str.Str.rsplit_once raw '@' = ok (some (user, s2)) ∧
                    s = s2)) ∧
              ((∃ (rest : Str),
                  core.str.Str.strip_prefix s
                      (toStr "[" split_host_port._proof_1) = ok (some rest) ∧
                    ((∃ (end1 : Usize) (end2 : Usize) (after : Str),
                        core.str.Str.find rest ']' = ok (some end1) ∧
                          (end1 + 1#usize) = ok end2 ∧
                            Str.Insts.CoreOpsIndexIndex.index
                                core.ops.range.RangeToInclusiveUsize.Insts.CoreSliceIndexSliceIndexStrStr
                                s { «end» := end2 } = ok h ∧
                              Str.Insts.CoreOpsIndexIndex.index
                                  core.ops.range.RangeFromUsize.Insts.CoreSliceIndexSliceIndexStrStr
                                  rest { start := end2 } = ok after ∧
                                ((core.str.Str.strip_prefix after
                                      (toStr ":" split_host_port._proof_2) =
                                    ok none ∧ p = none) ∨
                                  (∃ (p' : Str),
                                      core.str.Str.strip_prefix after
                                          (toStr ":" split_host_port._proof_2) =
                                        ok (some p') ∧
                                        ((core.str.Str.is_empty p' = ok true ∧
                                            p = none) ∨
                                          (core.str.Str.is_empty p' = ok false ∧
                                            p = some p')))))
                      ∨ (core.str.Str.find rest ']' = ok none ∧
                          split_host_port_colon s = ok (h, p))))
                ∨ (core.str.Str.strip_prefix s
                      (toStr "[" split_host_port._proof_1) = ok none ∧
                    split_host_port_colon s = ok (h, p)))) := by
  intro raw h p
  constructor
  · intro hval
    unfold split_host_port at hval
    obtain ⟨o, hro, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨s, hsO, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨obr, hbrO, hval⟩ := bind_ok_inv _ _ _ hval
    cases o with
    | none =>
      simp only at hsO
      have hs : s = raw := (Result.ok.inj hsO).symm
      refine ⟨s, ?_, ?_⟩
      · left; exact ⟨hro, hs⟩
      · cases obr with
        | some rest =>
          simp only at hval
          obtain ⟨oend, hfind, hval⟩ := bind_ok_inv _ _ _ hval
          cases oend with
          | some end1 =>
            simp only at hval
            obtain ⟨end2, hend2, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨host, hhost, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨after, hafter, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨oport, hport, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨port, hportM, hval⟩ := bind_ok_inv _ _ _ hval
            have hp2 := Result.ok.inj hval
            simp only [Prod.mk.injEq] at hp2
            obtain ⟨hh, hpq⟩ := hp2
            subst hh
            subst hpq
            left
            refine ⟨rest, hbrO, ?_⟩
            · left
              refine ⟨end1, end2, after, hfind, hend2, hhost, hafter, ?_⟩
              cases oport with
              | none =>
                simp only at hportM
                have hpn : port = none := (Result.ok.inj hportM).symm
                subst hpn
                left; exact ⟨hport, rfl⟩
              | some p' =>
                simp only at hportM
                obtain ⟨e, he, hvalM⟩ := bind_ok_inv _ _ _ hportM
                split at hvalM
                · next het =>
                  rw [het] at he
                  have hpn : port = none := (Result.ok.inj hvalM).symm
                  subst hpn
                  right; refine ⟨p', hport, ?_⟩; left; exact ⟨he, rfl⟩
                · next hef =>
                  have hef' : e = false := by simp at hef; exact hef
                  rw [hef'] at he
                  have hpn : port = some p' := (Result.ok.inj hvalM).symm
                  subst hpn
                  right; refine ⟨p', hport, ?_⟩; right; exact ⟨he, rfl⟩
          | none =>
            simp only at hval
            left
            refine ⟨rest, hbrO, ?_⟩
            right; exact ⟨hfind, hval⟩
        | none =>
          simp only at hval
          right; exact ⟨hbrO, hval⟩
    | some pair =>
      obtain ⟨user, s2⟩ := pair
      simp only at hsO
      have hs : s = s2 := (Result.ok.inj hsO).symm
      refine ⟨s, ?_, ?_⟩
      · right; exact ⟨user, s2, hro, hs⟩
      · cases obr with
        | some rest =>
          simp only at hval
          obtain ⟨oend, hfind, hval⟩ := bind_ok_inv _ _ _ hval
          cases oend with
          | some end1 =>
            simp only at hval
            obtain ⟨end2, hend2, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨host, hhost, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨after, hafter, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨oport, hport, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨port, hportM, hval⟩ := bind_ok_inv _ _ _ hval
            have hp2 := Result.ok.inj hval
            simp only [Prod.mk.injEq] at hp2
            obtain ⟨hh, hpq⟩ := hp2
            subst hh
            subst hpq
            left
            refine ⟨rest, hbrO, ?_⟩
            · left
              refine ⟨end1, end2, after, hfind, hend2, hhost, hafter, ?_⟩
              cases oport with
              | none =>
                simp only at hportM
                have hpn : port = none := (Result.ok.inj hportM).symm
                subst hpn
                left; exact ⟨hport, rfl⟩
              | some p' =>
                simp only at hportM
                obtain ⟨e, he, hvalM⟩ := bind_ok_inv _ _ _ hportM
                split at hvalM
                · next het =>
                  rw [het] at he
                  have hpn : port = none := (Result.ok.inj hvalM).symm
                  subst hpn
                  right; refine ⟨p', hport, ?_⟩; left; exact ⟨he, rfl⟩
                · next hef =>
                  have hef' : e = false := by simp at hef; exact hef
                  rw [hef'] at he
                  have hpn : port = some p' := (Result.ok.inj hvalM).symm
                  subst hpn
                  right; refine ⟨p', hport, ?_⟩; right; exact ⟨he, rfl⟩
          | none =>
            simp only at hval
            left
            refine ⟨rest, hbrO, ?_⟩
            right; exact ⟨hfind, hval⟩
        | none =>
          simp only at hval
          right; exact ⟨hbrO, hval⟩
  · rintro ⟨s, (⟨hro, rfl⟩ | ⟨user, s2, hro, rfl⟩),
      (⟨rest, hbr,
        (⟨end1, end2, after, hfind, hend2, hhost, hafter,
            (⟨hpo, rfl⟩ | ⟨p', hpo, (⟨he, rfl⟩ | ⟨he, rfl⟩)⟩)⟩ |
          ⟨hf, hcol⟩)⟩ |
        ⟨hbr, hcol⟩)⟩
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hfind]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hend2]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hhost]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hafter]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpo]; simp only [Aeneas.Std.bind_tc_ok]
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hfind]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hend2]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hhost]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hafter]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpo]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hfind]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hend2]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hhost]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hafter]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpo]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hf]; simp only [Aeneas.Std.bind_tc_ok]
      exact hcol
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      exact hcol
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hfind]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hend2]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hhost]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hafter]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpo]; simp only [Aeneas.Std.bind_tc_ok]
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hfind]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hend2]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hhost]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hafter]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpo]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hfind]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hend2]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hhost]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hafter]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hpo]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hf]; simp only [Aeneas.Std.bind_tc_ok]
      exact hcol
    · unfold split_host_port
      rw [hro]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      exact hcol

/-- RFC-0216 P2.1 8/8 (atom `catalog:request_target_authority`): a
  autoridade da request-target é exatamente a cadeia citada — strip do
  fragmento, o rest da autoridade HTTP (ou o fallback `//` quando não há
  scheme), e o corte no primeiro `/`/`?` (ou o fim); autoridade vazia
  rejeita (none). O `?` do strip_prefix fica citado pelo par opaco
  (branch/from_residual) do extrato Charon. -/
theorem request_target_authority_fate_iff :
    ∀ (t : Str) (r : Option Str),
      (request_target_authority t = ok r) ↔
        (∃ (target1 : Str),
            strip_uri_fragment t = ok target1 ∧
              ((∃ (rest : Str) (o : Option Usize) (i : Usize) (end1 : Usize) (auth : Str),
                    strip_http_authority_rest target1 = ok (some rest) ∧
                      core.str.Str.find rest (Array.make 2#usize [ '/', '?' ]) = ok o ∧
                        core.str.Str.len rest = ok i ∧
                          lift (core.option.Option.unwrap_or o i) = ok end1 ∧
                            Str.Insts.CoreOpsIndexIndex.index
                                core.ops.range.RangeToUsize.Insts.CoreSliceIndexSliceIndexStrStr
                                rest { «end» := end1 } = ok auth ∧
                              ((core.str.Str.is_empty auth = ok true ∧ r = none) ∨
                                (core.str.Str.is_empty auth = ok false ∧ r = some auth))) ∨
                (strip_http_authority_rest target1 = ok none ∧
                  ∃ (o : Option Str)
                      (cf : core.ops.control_flow.ControlFlow
                          (Option core.convert.Infallible) Str),
                    core.str.Str.strip_prefix target1
                        (toStr "//" request_target_authority._proof_1) =
                      ok o ∧
                      core.option.Option.Insts.CoreOpsTry_traitTry.branch o = ok cf ∧
                        ((∃ (val : Str) (o2 : Option Usize) (i : Usize) (end1 : Usize) (auth : Str),
                              cf = core.ops.control_flow.ControlFlow.Continue val ∧
                                core.str.Str.find val (Array.make 2#usize [ '/', '?' ]) =
                                  ok o2 ∧
                                  core.str.Str.len val = ok i ∧
                                    lift (core.option.Option.unwrap_or o2 i) = ok end1 ∧
                                      Str.Insts.CoreOpsIndexIndex.index
                                          core.ops.range.RangeToUsize.Insts.CoreSliceIndexSliceIndexStrStr
                                          val { «end» := end1 } = ok auth ∧
                                        ((core.str.Str.is_empty auth = ok true ∧ r = none) ∨
                                          (core.str.Str.is_empty auth = ok false ∧
                                            r = some auth))) ∨
                          (∃ (res : Option core.convert.Infallible),
                              cf = core.ops.control_flow.ControlFlow.Break res ∧
                                core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual
                                  Str res = ok r))))) := by
  intro t r
  constructor
  · intro hval
    unfold request_target_authority at hval
    obtain ⟨target1, ht1, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨o, ho, hval⟩ := bind_ok_inv _ _ _ hval
    cases o with
    | some rest =>
      dsimp only at hval
      obtain ⟨o1, hf, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨i, hl, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨end1, he, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨auth, ha, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
      refine ⟨target1, ht1, ?_⟩
      left
      refine ⟨rest, o1, i, end1, auth, ho, hf, hl, he, ha, ?_⟩
      split at hval
      · next hbt =>
        rw [hbt] at hb
        left; exact ⟨hb, (Result.ok.inj hval).symm⟩
      · next hbf =>
        have hbf' : b = false := by simp at hbf; exact hbf
        rw [hbf'] at hb
        right; exact ⟨hb, (Result.ok.inj hval).symm⟩
    | none =>
      dsimp only at hval
      obtain ⟨o1, hsp, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨cf, hbr, hval⟩ := bind_ok_inv _ _ _ hval
      refine ⟨target1, ht1, ?_⟩
      right
      refine ⟨ho, o1, cf, hsp, hbr, ?_⟩
      cases cf with
      | Continue val =>
        dsimp only at hval
        obtain ⟨o2, hf, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨i, hl, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨end1, he, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨auth, ha, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
        left
        refine ⟨val, o2, i, end1, auth, rfl, hf, hl, he, ha, ?_⟩
        split at hval
        · next hbt =>
          rw [hbt] at hb
          left; exact ⟨hb, (Result.ok.inj hval).symm⟩
        · next hbf =>
          have hbf' : b = false := by simp at hbf; exact hbf
          rw [hbf'] at hb
          right; exact ⟨hb, (Result.ok.inj hval).symm⟩
      | Break res =>
        dsimp only at hval
        right; exact ⟨res, rfl, hval⟩
  · rintro ⟨target1, ht1,
      (⟨rest, o, i, end1, auth, ho, hf, hl, he, ha, (⟨hbt, rfl⟩ | ⟨hbf, rfl⟩)⟩ |
        ⟨ho, o, cf, hsp, hbr,
          (⟨val, o2, i, end1, auth, hcf, hf, hl, he, ha, (⟨hbt, rfl⟩ | ⟨hbf, rfl⟩)⟩ |
            ⟨res, hcf, hres⟩)⟩)⟩
    · unfold request_target_authority
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hf]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hl]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ha]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbt]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold request_target_authority
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hf]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hl]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ha]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbf]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold request_target_authority
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hsp]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hcf]; dsimp only
      rw [hf]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hl]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ha]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbt]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold request_target_authority
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hsp]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hcf]; dsimp only
      rw [hf]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hl]; simp only [Aeneas.Std.bind_tc_ok]
      rw [he]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ha]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbf]; simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold request_target_authority
      rw [ht1]; simp only [Aeneas.Std.bind_tc_ok]
      rw [ho]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hsp]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hbr]; simp only [Aeneas.Std.bind_tc_ok]
      rw [hcf]
      exact hres
