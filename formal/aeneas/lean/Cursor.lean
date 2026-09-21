-- Theorems over Aeneas extract of cursor_kernel.rs
import Aeneas
import CursorKernel
open Aeneas.Std Result
open pedra_aeneas_cursor_kernel

theorem next_seq_from_zero :
    next_seq 0#u64 = ok (1#u64) := by
  unfold next_seq
  have h : core.num.U64.saturating_add 0#u64 1#u64 = 1#u64 := by native_decide
  simp [h]

/-- RFC-0218 P2.1 5/12 (atom `catalog:stream_next_seq`, entrada
    `next_seq`): a próxima sequência é EXATAMENTE o lift citado
    `saturating_add last_acked 1` — o ack anda uma casa sem overflow.
    O AS-IS devolve o próprio last_acked (ack não avança — tooth
    plantado). -/
theorem next_seq_fate_iff :
    ∀ (last_acked : U64) (r : U64),
      (next_seq last_acked = ok r) ↔
      (r = core.num.U64.saturating_add last_acked 1#u64) := by
  intro last_acked r
  constructor
  · intro hval
    unfold next_seq at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
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

/-- RFC-0218 P2.1 6/12 (atom `catalog:stream_cursor`, entrada
    `ack_in_order`): ack em ordem é EXATAMENTE a cadeia citada — o
    cursor esperado é `next_seq last_acked` (gate), e o ack conta só
    quando a sequência é exatamente a esperada E está à frente do
    último ack. O AS-IS aceita qualquer sequência à frente (pula
    buracos — tooth plantado). -/
theorem ack_in_order_fate_iff :
    ∀ (last_acked : U64) (seq : U64) (v : Bool),
      (ack_in_order last_acked seq = ok v) ↔
        (∃ i : U64, next_seq last_acked = ok i ∧
          ((seq = i ∧ v = decide (seq > last_acked)) ∨
           (¬ (seq = i) ∧ v = false))) := by
  intro last_acked seq v
  constructor
  · intro hval
    unfold ack_in_order at hval
    obtain ⟨i, hg, hval⟩ := bind_ok_inv _ _ _ hval
    refine ⟨i, hg, ?_⟩
    split at hval
    · next hc =>
      injection hval with hv
      exact Or.inl ⟨hc, hv.symm⟩
    · next hc =>
      injection hval with hv
      exact Or.inr ⟨hc, hv.symm⟩
  · rintro ⟨i, hg, (⟨hc, hv⟩ | ⟨hc, hv⟩)⟩
    · subst hv
      unfold ack_in_order
      refine bind_intro i hg ?_
      rw [if_pos hc]
    · subst hv
      unfold ack_in_order
      refine bind_intro i hg ?_
      rw [if_neg (by simp [hc])]
