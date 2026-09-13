-- Theorems over the Aeneas extract of production wal/recover_kernel.rs
-- (RFC-0053 P1.3 / RFC-0056 P1.2): second machine (not the Verus twin).
import Aeneas
import WalRecoverKernel
open Aeneas Std Result
open pedra_aeneas_wal_recover_kernel

/-- A complete record is always kept — the collector never drops a
record silently. -/
theorem record_is_kept (prefix_n : Std.U64) (can_skip : Bool)
    (skips : Std.U64) (in_resync : Bool) :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Record
      prefix_n can_skip skips in_resync =
      ok recover_kernel.RecoverAct.KeepRecord := by
  rfl

/-- G8: CRC at a fresh alignment fail-stops (a real record with a bad
checksum is never skipped). -/
theorem crc_fresh_alignment_fail_stops (prefix_n : Std.U64) (can_skip : Bool)
    (skips : Std.U64) :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Crc
      prefix_n can_skip skips false =
      ok recover_kernel.RecoverAct.FailStop := by
  cases can_skip <;> rfl

/-- F4: torn first record on an empty prefix fail-stops (not a silent
empty WAL). -/
theorem empty_prefix_torn_fail_stops :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Truncated
      (0#u64) false (0#u64) false =
      ok recover_kernel.RecoverAct.FailStop := by
  rfl

/-- A torn tail over a live prefix keeps the prefix (the discard is the
prefix boundary, not the whole log). -/
theorem prefix_torn_keeps :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Truncated
      (1#u64) false (0#u64) false =
      ok recover_kernel.RecoverAct.KeepPrefix := by
  rfl

/-- F14: an orphan fragment fail-stops (never clean EOF). -/
theorem orphan_fragment_fail_stops :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.OrphanFragment
      (1#u64) true (0#u64) false =
      ok recover_kernel.RecoverAct.FailStop := by
  rfl

/-- AS-IS teeth: torn empty prefix becomes a silent Stop (empty WAL)
where the fixed kernel fail-stops — the F4 silent-wrong. -/
theorem as_is_torn_is_silent_eof :
    recover_kernel.recover_collect_act_as_is recover_kernel.RecoverKind.Truncated
      (0#u64) false (0#u64) =
      ok recover_kernel.RecoverAct.Stop ∧
      recover_kernel.recover_collect_act recover_kernel.RecoverKind.Truncated
        (0#u64) false (0#u64) false =
        ok recover_kernel.RecoverAct.FailStop := by
  constructor <;> rfl

/-- AS-IS teeth: CRC at a fresh alignment silently resyncs where the
fixed kernel fail-stops — the G8 silent-wrong. -/
theorem as_is_crc_resyncs :
    recover_kernel.recover_collect_act_as_is recover_kernel.RecoverKind.Crc
      (3#u64) true (0#u64) =
      ok recover_kernel.RecoverAct.Resync ∧
      recover_kernel.recover_collect_act recover_kernel.RecoverKind.Crc
        (3#u64) true (0#u64) false =
        ok recover_kernel.RecoverAct.FailStop := by
  constructor <;> rfl

/-- AS-IS teeth: `is_length_resyncable_as_is` misclassifies CRC as
length-resyncable — the misclassification the fixed classifier refuses. -/
theorem as_is_crc_not_length_resyncable :
    recover_kernel.is_length_resyncable recover_kernel.RecoverKind.Crc = ok false ∧
      recover_kernel.is_length_resyncable_as_is recover_kernel.RecoverKind.Crc = ok true := by
  constructor <;> rfl


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
/-- RFC-0213 P2.1 (wal finais 1/2): the recovery collector's action is
    decided EXACTLY by the extracted match — every one of the nine
    framing kinds maps to its fate along the route rustc links: a
    record is always kept, a clean EOF stops, the torn trio
    (Truncated/LengthCorrupt/UnknownType) fail-stops an empty prefix,
    keeps a live prefix, or resyncs under the skip budget measured by
    the extracted MAX_CONSECUTIVE_SKIPS (fail-stop past it), a CRC or
    zero-header tail mid-resync-walk resyncs/keeps the prefix and
    fail-stops at a fresh alignment, orphan fragments and unknown
    damage always fail-stop. The AS-IS mutant calls a torn tail a
    clean EOF (silent prefix loss) and resyncs a bad CRC — refuted
    live by `recover_collect_act_on_live_exploded_crc_is_not_ok`. -/
theorem recover_collect_act_fate_iff :
    ∀ (kind : recover_kernel.RecoverKind) (prefix_n : Std.U64)
      (can_skip : Bool) (consecutive_skips : Std.U64) (in_resync : Bool)
      (v : recover_kernel.RecoverAct),
      (recover_kernel.recover_collect_act kind prefix_n can_skip
          consecutive_skips in_resync = ok v) ↔
        ((kind = recover_kernel.RecoverKind.Record ∧
            v = recover_kernel.RecoverAct.KeepRecord) ∨
          (kind = recover_kernel.RecoverKind.CleanEof ∧
            v = recover_kernel.RecoverAct.Stop) ∨
          ((kind = recover_kernel.RecoverKind.Truncated ∨
              kind = recover_kernel.RecoverKind.LengthCorrupt ∨
              kind = recover_kernel.RecoverKind.UnknownType) ∧
            ((¬(can_skip = true) ∧
                ((prefix_n = 0#u64 ∧ v = recover_kernel.RecoverAct.FailStop) ∨
                  (¬(prefix_n = 0#u64) ∧
                    v = recover_kernel.RecoverAct.KeepPrefix))) ∨
              (can_skip = true ∧
                ∃ i, recover_kernel.MAX_CONSECUTIVE_SKIPS = ok i ∧
                  ((consecutive_skips > i ∧
                      v = recover_kernel.RecoverAct.FailStop) ∨
                    (¬(consecutive_skips > i) ∧
                      v = recover_kernel.RecoverAct.Resync))))) ∨
          (kind = recover_kernel.RecoverKind.OrphanFragment ∧
            v = recover_kernel.RecoverAct.FailStop) ∨
          ((kind = recover_kernel.RecoverKind.Crc ∨
              kind = recover_kernel.RecoverKind.ZeroHeaderTail) ∧
            ((in_resync = true ∧
                ((can_skip = true ∧ v = recover_kernel.RecoverAct.Resync) ∨
                  (¬(can_skip = true) ∧
                    ((prefix_n = 0#u64 ∧
                        v = recover_kernel.RecoverAct.FailStop) ∨
                      (¬(prefix_n = 0#u64) ∧
                        v = recover_kernel.RecoverAct.KeepPrefix))))) ∨
              (¬(in_resync = true) ∧
                v = recover_kernel.RecoverAct.FailStop))) ∨
          (kind = recover_kernel.RecoverKind.Other ∧
            v = recover_kernel.RecoverAct.FailStop)) := by
  intro kind prefix_n can_skip consecutive_skips in_resync v
  cases kind with
  | Record =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    · rintro (⟨-, hv⟩ | ⟨heq, -⟩ | ⟨htrio, -⟩ | ⟨heq, -⟩ | ⟨hcrc, -⟩ |
        ⟨heq, -⟩)
      · subst hv
        rfl
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases htrio with heq | heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases hcrc with heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
  | CleanEof =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inl ⟨rfl, hv.symm⟩)
    · rintro (⟨heq, -⟩ | ⟨-, hv⟩ | ⟨htrio, -⟩ | ⟨heq, -⟩ | ⟨hcrc, -⟩ |
        ⟨heq, -⟩)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · rcases htrio with heq | heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases hcrc with heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
  | Truncated | LengthCorrupt | UnknownType =>
    constructor
    · intro hval
      unfold recover_kernel.recover_collect_act at hval
      split at hval
      all_goals first
        | next heq =>
          exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
        | next _ =>
          split at hval
          · next hcs =>
            obtain ⟨i, hmax, hval⟩ := bind_ok_inv _ _ _ hval
            refine Or.inr (Or.inr (Or.inl ⟨?_, Or.inr ⟨hcs, i, hmax, ?_⟩⟩))
            · first
              | exact Or.inl rfl
              | exact Or.inr (Or.inl rfl)
              | exact Or.inr (Or.inr rfl)
            · split at hval
              · next hgt =>
                injection hval with hv
                exact Or.inl ⟨hgt, hv.symm⟩
              · next hgt =>
                injection hval with hv
                exact Or.inr ⟨hgt, hv.symm⟩
          · next hns =>
            refine Or.inr (Or.inr (Or.inl ⟨?_, Or.inl ⟨hns, ?_⟩⟩))
            · first
              | exact Or.inl rfl
              | exact Or.inr (Or.inl rfl)
              | exact Or.inr (Or.inr rfl)
            · split at hval
              · next hp =>
                injection hval with hv
                exact Or.inl ⟨hp, hv.symm⟩
              · next hp =>
                injection hval with hv
                exact Or.inr ⟨hp, hv.symm⟩
    · rintro (⟨heq, -⟩ | ⟨heq, -⟩ | ⟨htrio, hlast⟩ | ⟨heq, -⟩ |
        ⟨hcrc, -⟩ | ⟨heq, -⟩)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases htrio with heq | heq | heq
        all_goals first
          | exact absurd heq
              (fun h => recover_kernel.RecoverKind.noConfusion h)
          | unfold recover_kernel.recover_collect_act
            split
            all_goals first
              | next heq =>
          exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
              | next _ =>
                rcases hlast with ⟨hns, hp⟩ | ⟨hcs, i, hmax, hgt⟩
                · rcases hp with ⟨hp, hv⟩ | ⟨hp, hv⟩
                  · rw [if_neg hns, if_pos hp, hv]
                  · rw [if_neg hns, if_neg hp, hv]
                · rw [if_pos hcs]
                  refine bind_intro i hmax ?_
                  rcases hgt with ⟨hgt, hv⟩ | ⟨hgt, hv⟩
                  · rw [if_pos hgt, hv]
                  · rw [if_neg hgt, hv]
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases hcrc with heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
  | OrphanFragment =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩)))
    · rintro (⟨heq, -⟩ | ⟨heq, -⟩ | ⟨htrio, -⟩ | ⟨-, hv⟩ | ⟨hcrc, -⟩ |
        ⟨heq, -⟩)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases htrio with heq | heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · rcases hcrc with heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
  | Crc | ZeroHeaderTail =>
    constructor
    · intro hval
      unfold recover_kernel.recover_collect_act at hval
      split at hval
      all_goals first
        | next heq =>
          exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
        | next _ =>
          split at hval
          · next hrs =>
            refine Or.inr (Or.inr (Or.inr (Or.inr
                (Or.inl ⟨?_, Or.inl ⟨hrs, ?_⟩⟩))))
            · first
              | exact Or.inl rfl
              | exact Or.inr rfl
            · split at hval
              · next hcs =>
                injection hval with hv
                exact Or.inl ⟨hcs, hv.symm⟩
              · next hns =>
                refine Or.inr ⟨hns, ?_⟩
                split at hval
                · next hp =>
                  injection hval with hv
                  exact Or.inl ⟨hp, hv.symm⟩
                · next hp =>
                  injection hval with hv
                  exact Or.inr ⟨hp, hv.symm⟩
          · next hrs =>
            injection hval with hv
            refine Or.inr (Or.inr (Or.inr (Or.inr
              (Or.inl ⟨?_, Or.inr ⟨hrs, hv.symm⟩⟩))))
            first
            | exact Or.inl rfl
            | exact Or.inr rfl
    · rintro (⟨heq, -⟩ | ⟨heq, -⟩ | ⟨htrio, -⟩ | ⟨heq, -⟩ |
        ⟨hcrc, hlast⟩ | ⟨heq, -⟩)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases htrio with heq | heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases hcrc with heq | heq
        all_goals first
          | exact absurd heq
              (fun h => recover_kernel.RecoverKind.noConfusion h)
          | unfold recover_kernel.recover_collect_act
            split
            all_goals first
              | next heq =>
          exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
              | next _ =>
                rcases hlast with ⟨hrs, hlast⟩ | ⟨hrs, hv⟩
                · rw [if_pos hrs]
                  rcases hlast with ⟨hcs, hv⟩ | ⟨hns, hp⟩
                  · rw [if_pos hcs, hv]
                  · rw [if_neg hns]
                    rcases hp with ⟨hp, hv⟩ | ⟨hp, hv⟩
                    · rw [if_pos hp, hv]
                    · rw [if_neg hp, hv]
                · rw [if_neg hrs, hv]
      · exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
  | Other =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inr ⟨rfl, hv.symm⟩))))
    · rintro h
      rcases h with h1 | h1
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      rcases h1 with h2 | h2
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      rcases h2 with h3 | h3
      · rcases h3.1 with heq | heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      rcases h3 with h4 | h4
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      rcases h4 with h5 | h6
      · rcases h5.1 with heq | heq <;>
        exact absurd heq (fun h => recover_kernel.RecoverKind.noConfusion h)
      · rcases h6 with ⟨_, hv⟩
        subst hv
        rfl

/-- RFC-0218 P0.2 1/4 (átomo `catalog:from_record_type`): o tipo de
    fragmento é EXATAMENTE a bijeção total do RecordType do wire —
    cada um dos 5 tipos mapeia para o seu FragKind, sem terceiro
    destino. O AS-IS reclassifica First como Middle (dente: fragmento
    de início vira meio — a planta on-wire recusa). -/
theorem from_record_type_fate_iff :
    ∀ (t : format.RecordType) (f : recover_kernel.FragKind),
      (recover_kernel.FragKind.from_record_type t = ok f) ↔
        ((t = format.RecordType.Zero ∧ f = recover_kernel.FragKind.Zero) ∨
          (t = format.RecordType.Full ∧ f = recover_kernel.FragKind.Full) ∨
          (t = format.RecordType.First ∧ f = recover_kernel.FragKind.First) ∨
          (t = format.RecordType.Middle ∧ f = recover_kernel.FragKind.Middle) ∨
          (t = format.RecordType.Last ∧ f = recover_kernel.FragKind.Last)) := by
  intro t f
  cases t with
  | Zero =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2 | h3 | h4 | h5)
      · subst hv
        rfl
      · exact absurd h2.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h3.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h4.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h5.1 (fun h => format.RecordType.noConfusion h)
  | Full =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inl ⟨rfl, hv.symm⟩)
    · rintro (h1 | ⟨-, hv⟩ | h3 | h4 | h5)
      · exact absurd h1.1 (fun h => format.RecordType.noConfusion h)
      · subst hv
        rfl
      · exact absurd h3.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h4.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h5.1 (fun h => format.RecordType.noConfusion h)
  | First =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩))
    · rintro (h1 | h2 | ⟨-, hv⟩ | h4 | h5)
      · exact absurd h1.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h2.1 (fun h => format.RecordType.noConfusion h)
      · subst hv
        rfl
      · exact absurd h4.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h5.1 (fun h => format.RecordType.noConfusion h)
  | Middle =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩)))
    · rintro (h1 | h2 | h3 | ⟨-, hv⟩ | h5)
      · exact absurd h1.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h2.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h3.1 (fun h => format.RecordType.noConfusion h)
      · subst hv
        rfl
      · exact absurd h5.1 (fun h => format.RecordType.noConfusion h)
  | Last =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inr ⟨rfl, hv.symm⟩)))
    · rintro (h1 | h2 | h3 | h4 | ⟨-, hv⟩)
      · exact absurd h1.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h2.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h3.1 (fun h => format.RecordType.noConfusion h)
      · exact absurd h4.1 (fun h => format.RecordType.noConfusion h)
      · subst hv
        rfl

/-- RFC-0218 P0.2 2/4 (átomo `catalog:is_length_resyncable`): a
    classe de resync é EXATAMENTE o trio de dano de comprimento
    (Truncated/LengthCorrupt/UnknownType) — os seis demais tipos nunca
    são resyncable por comprimento. O AS-IS promove Crc a
    length-resyncable (misclassificação que o fixo recusa — dente
    plantado). -/
theorem is_length_resyncable_fate_iff :
    ∀ (kind : recover_kernel.RecoverKind) (v : Bool),
      (recover_kernel.is_length_resyncable kind = ok v) ↔
        ((kind = recover_kernel.RecoverKind.Truncated ∧ v = true) ∨
          (kind = recover_kernel.RecoverKind.LengthCorrupt ∧ v = true) ∨
          (kind = recover_kernel.RecoverKind.UnknownType ∧ v = true) ∨
          (kind = recover_kernel.RecoverKind.Record ∧ v = false) ∨
          (kind = recover_kernel.RecoverKind.CleanEof ∧ v = false) ∨
          (kind = recover_kernel.RecoverKind.OrphanFragment ∧ v = false) ∨
          (kind = recover_kernel.RecoverKind.Crc ∧ v = false) ∨
          (kind = recover_kernel.RecoverKind.ZeroHeaderTail ∧ v = false) ∨
          (kind = recover_kernel.RecoverKind.Other ∧ v = false)) := by
  intro kind v
  cases kind with
  | Truncated =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2 | h3 | h4 | h5 | h6 | h7 | h8 | h9)
      · subst hv
        rfl
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | LengthCorrupt =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inl ⟨rfl, hv.symm⟩)
    · rintro (h1 | ⟨-, hv⟩ | h3 | h4 | h5 | h6 | h7 | h8 | h9)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | UnknownType =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩))
    · rintro (h1 | h2 | ⟨-, hv⟩ | h4 | h5 | h6 | h7 | h8 | h9)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | Record =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩)))
    · rintro (h1 | h2 | h3 | ⟨-, hv⟩ | h5 | h6 | h7 | h8 | h9)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | CleanEof =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩))))
    · rintro (h1 | h2 | h3 | h4 | ⟨-, hv⟩ | h6 | h7 | h8 | h9)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | OrphanFragment =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩)))))
    · rintro (h1 | h2 | h3 | h4 | h5 | ⟨-, hv⟩ | h7 | h8 | h9)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | Crc =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩))))))
    · rintro (h1 | h2 | h3 | h4 | h5 | h6 | ⟨-, hv⟩ | h8 | h9)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | ZeroHeaderTail =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, hv.symm⟩)))))))
    · rintro (h1 | h2 | h3 | h4 | h5 | h6 | h7 | ⟨-, hv⟩ | h9)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
      · exact absurd h9.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
  | Other =>
    constructor
    · intro hval
      injection hval with hv
      refine Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inr (Or.inr ?_)))))))
      exact ⟨rfl, hv.symm⟩
    · rintro (h1 | h2 | h3 | h4 | h5 | h6 | h7 | h8 | ⟨-, hv⟩)
      · exact absurd h1.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h2.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h3.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h4.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h5.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h6.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h7.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · exact absurd h8.1 (fun h => recover_kernel.RecoverKind.noConfusion h)
      · subst hv
        rfl
