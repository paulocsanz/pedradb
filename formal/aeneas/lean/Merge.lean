-- Theorems over Aeneas extract of merge.rs visible_at (RFC-0150 / F30)
-- plus user_key_in_range / past_end. WindowKvIter is Iterator-refused.
-- RFC-0187 P1.3 / RFC-0188 P0.2: the heap-sift STRUCTURE kernel
-- (sift_step) — first `close` of the depth ladder (RFC-0188).
-- RFC-0191 P2.2: the one-step Inv-LSM lemma composes this get atom with
-- the 0164 probe-order kernel (newest-first tie-break).
import Aeneas
import MergeKernel
import ProbeOrder
open Aeneas.Std Result
open pedra_aeneas_merge_kernel

/-- RFC-0188 first `close` (named property, all inputs): the sift kernel
stays put EXACTLY when no repair is needed — the best child does not
beat the hole. The as-is mutant stays even when repair is needed. -/
theorem merge_sift_step_repairs_iff :
    ∀ (r_exists r_lt_l best_lt_hole : Bool),
      (merge.sift_step r_exists r_lt_l best_lt_hole
        = ok merge.SiftStep.Stay) ↔ (best_lt_hole = false) := by
  intro r_exists r_lt_l best_lt_hole
  unfold merge.sift_step
  cases r_exists <;> cases r_lt_l <;> cases best_lt_hole <;> simp

/-- The right child wins the swap EXACTLY when a repair is needed, the
right child exists, and it beats the left child. -/
theorem merge_sift_step_swap_right_iff :
    ∀ (r_exists r_lt_l best_lt_hole : Bool),
      (merge.sift_step r_exists r_lt_l best_lt_hole
        = ok merge.SiftStep.SwapRight)
        ↔ (best_lt_hole = true ∧ r_exists = true ∧ r_lt_l = true) := by
  intro r_exists r_lt_l best_lt_hole
  unfold merge.sift_step
  cases r_exists <;> cases r_lt_l <;> cases best_lt_hole <;> simp

/-- AS-IS dente (Lean side): on every repairing input the mutant stays
and the kernel does not — the decisions diverge. -/
theorem merge_sift_step_as_is_diverges_on_repair (r_exists r_lt_l : Bool) :
    merge.sift_step_as_is r_exists r_lt_l true
      ≠ merge.sift_step r_exists r_lt_l true := by
  unfold merge.sift_step merge.sift_step_as_is
  cases r_exists <;> cases r_lt_l <;> simp

/-- Catalog entry: a deletion is never live. -/
theorem visible_at_deletion :
    merge.visible_at key.ValueType.Deletion false = ok false := by
  unfold merge.visible_at
  rfl

/-- RFC-0188 P1.6 first `atom` (data-fate rule, all inputs): a deletion
never surfaces live, WHATEVER the covering-range says — the fate of the
deleted version is decided by the kind alone. -/
theorem visible_at_deletion_never_live :
    ∀ (range_hidden : Bool),
      merge.visible_at key.ValueType.Deletion range_hidden = ok false := by
  intro range_hidden
  unfold merge.visible_at
  cases range_hidden <;> rfl

/-- RFC-0191 P0.2 product corollary R1-deletion: a Deletion never
surfaces live. Unfolds production `visible_at` (the get-path atom). -/
theorem r1_deletion_never_live :
    ∀ (range_hidden : Bool),
      merge.visible_at key.ValueType.Deletion range_hidden = ok false := by
  intro range_hidden
  unfold merge.visible_at
  cases range_hidden <;> rfl

/-- RFC-0191 P1.1 product corollary R1 both arms: deletion never live,
Value live iff not hidden. Unfolds production `visible_at` twice. -/
theorem r1_get_atom :
    ∀ (range_hidden : Bool),
      (merge.visible_at key.ValueType.Deletion range_hidden = ok false)
      ∧ (merge.visible_at key.ValueType.Value range_hidden
          = ok (!range_hidden)) := by
  intro range_hidden
  constructor
  · unfold merge.visible_at; cases range_hidden <;> rfl
  · unfold merge.visible_at; cases range_hidden <;> rfl

/-- A Value surfaces exactly when no covering range hides it (all inputs). -/
theorem visible_at_value_live_iff_not_hidden :
    ∀ (range_hidden : Bool),
      merge.visible_at key.ValueType.Value range_hidden = ok (!range_hidden) := by
  intro range_hidden
  unfold merge.visible_at
  cases range_hidden <;> rfl

/-- RFC-0191 P2.2 one-step Inv-LSM: with the 0164 newest-first probe
order (equal-lo ties probe the newer table first), the first covering
version the get filter answers for is genuinely live — `visible_at`
never answers live for a Deletion or RangeDeletion (any cover) nor for
a hidden Value. Unfolds the registered R1 atom and the 0164 kernel. -/
theorem inv_lsm_newest_first_never_non_live :
    ∀ (newer older : Usize) (kind : key.ValueType) (range_hidden : Bool),
      pedra_aeneas_probe_order_kernel.first_probe_on_equal_lo newer older
        = ok newer →
      merge.visible_at kind range_hidden = ok true →
      kind = key.ValueType.Value ∧ range_hidden = false := by
  intro newer older kind range_hidden hnewest hlive
  unfold pedra_aeneas_probe_order_kernel.first_probe_on_equal_lo at hnewest
  unfold merge.visible_at at hlive
  cases kind with
  | Deletion => simp at hlive
  | RangeDeletion => simp at hlive
  | Value =>
      cases range_hidden with
      | true => simp at hlive
      | false => exact ⟨rfl, rfl⟩

/-- RFC-0191 P2.2 product corollary R1: get never returns a non-live
version. The Deletion arm is pinned by P0.2 (`r1_deletion_never_live`),
the hidden-Value arm by P1.1 (`r1_get_atom`), the newest-first premise
by the 0164 kernel theorem, and the honest arm by the one-step Inv-LSM
lemma. R1 stays `atom` (no layer move). -/
theorem r1_get_never_returns_non_live :
    ∀ (kind : key.ValueType) (range_hidden : Bool),
      merge.visible_at kind range_hidden = ok true →
      kind = key.ValueType.Value ∧ range_hidden = false := by
  intro kind range_hidden hlive
  have hnew : pedra_aeneas_probe_order_kernel.first_probe_on_equal_lo
      (1#usize) (0#usize) = ok (1#usize) :=
    first_probe_on_equal_lo_newer
  have hdel := r1_deletion_never_live range_hidden
  have hval := (r1_get_atom range_hidden).2
  cases kind with
  | Deletion => rw [hdel] at hlive; simp at hlive
  | RangeDeletion =>
      have hkill := inv_lsm_newest_first_never_non_live (1#usize) (0#usize)
        key.ValueType.RangeDeletion range_hidden hnew hlive
      exact absurd hkill.1 (by simp)
  | Value =>
      cases range_hidden with
      | true => rw [hval] at hlive; simp at hlive
      | false =>
          exact inv_lsm_newest_first_never_non_live (1#usize) (0#usize)
            key.ValueType.Value false hnew hlive

/-! ### RFC-0198 P1.3 — corolário indutivo Inv-LSM (cadeia de k merges)

A forma seL4 do Inv-LSM: o lema um-passo registrado cobre um topo de
heap; o corolário encadeia k passos por indução sobre a cadeia. O passo
CITA `inv_lsm_newest_first_never_non_live` (RFC-0191 P2.2) — nada é
re-provado aqui. -/

/-- Um passo da cadeia de merges: o par de idades no topo do heap
(newest primeiro no empate de chaves), o kind da versão que sobe e o
bool de range cobrindo a chave. -/
structure MergeStep where
  newer : Usize
  older : Usize
  kind : key.ValueType
  range_hidden : Bool

/-- Premissa estrutural do passo: o heap mantém a ordem newest-first —
no empate de chaves, o probe 0164 responde o mais novo primeiro. -/
def merge_step_newest_first (s : MergeStep) : Prop :=
  pedra_aeneas_probe_order_kernel.first_probe_on_equal_lo s.newer s.older
    = ok s.newer

/-- O filtro do get respondeu "live" para a versão que subiu neste
passo. -/
def merge_step_answers_live (s : MergeStep) : Prop :=
  merge.visible_at s.kind s.range_hidden = ok true

/-- Cadeia de k passos de merge. Base: cadeia vazia (k = 0 — nenhuma
versão subiu, vale trivialmente). Passo: um topo newest-first seguido
de uma cadeia de k passos — a premissa estrutural é do passo (o heap é
restaurado newest-first a cada saída), não de um par fixo. -/
inductive merge_chain : Nat → List MergeStep → Prop
  | nil : merge_chain 0 []
  | cons (s : MergeStep) (k : Nat) (rest : List MergeStep) :
      merge_step_newest_first s →
      merge_chain k rest →
      merge_chain (k + 1) (s :: rest)

/-- RFC-0198 P1.3 COROLÁRIO INDUTIVO: numa cadeia de k merges em que
todo topo permaneceu newest-first (premissa estrutural da cadeia),
TODO passo cujo filtro respondeu live é genuinamente live — Value não
escondido por range. Indução sobre a cadeia; o caso do passo CITA o
lema um-passo REGISTRADO `inv_lsm_newest_first_never_non_live`
(RFC-0191 P2.2). -/
theorem merge_chain_preserves_inv_lsm :
    ∀ (k : Nat) (chain : List MergeStep),
      merge_chain k chain →
      ∀ s ∈ chain,
        merge_step_answers_live s →
          s.kind = key.ValueType.Value ∧ s.range_hidden = false := by
  intro k chain hchain
  induction hchain with
  | nil => intro s hs; cases hs
  | cons s k' rest hnewest _ IH =>
      intro s hs hlive
      cases hs with
      | head =>
          exact inv_lsm_newest_first_never_non_live s.newer s.older s.kind
            s.range_hidden hnewest hlive
      | tail _ hs => exact IH s hs hlive

/-- Catalog entry: a Value is live unless a covering range hides it. -/
theorem visible_at_value_live :
    merge.visible_at key.ValueType.Value false = ok true := by
  unfold merge.visible_at
  rfl

/-- Catalog entry: a Value hidden by a covering range is not live. -/
theorem visible_at_value_hidden :
    merge.visible_at key.ValueType.Value true = ok false := by
  unfold merge.visible_at
  rfl

/-- Catalog entry: a range deletion is never live. -/
theorem visible_at_range_deletion :
    merge.visible_at key.ValueType.RangeDeletion false = ok false := by
  unfold merge.visible_at
  rfl

/-- AS-IS dente: a deletion still scans live. -/
theorem visible_at_as_is_dente :
    merge.visible_at_as_is key.ValueType.Deletion true = ok true := by
  unfold merge.visible_at_as_is
  rfl

/-- Unbounded window contains every key. -/
theorem user_key_in_range_unbounded (k) :
    merge.user_key_in_range k core.ops.range.Bound.Unbounded
      core.ops.range.Bound.Unbounded = ok true := by
  unfold merge.user_key_in_range
  rfl

/-- Exclusive start is slice `>` then unbounded end. Dual-unfold. -/
theorem user_key_in_range_excluded_unbounded (user s) :
    merge.user_key_in_range user (core.ops.range.Bound.Excluded s)
      core.ops.range.Bound.Unbounded =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        if after_start then ok true else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Inclusive start is slice `>=` then unbounded end. Dual-unfold. -/
theorem user_key_in_range_included_unbounded (user s) :
    merge.user_key_in_range user (core.ops.range.Bound.Included s)
      core.ops.range.Bound.Unbounded =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        if after_start then ok true else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Exclusive end, unbounded start: key must be `<` end. Dual-unfold. -/
theorem user_key_in_range_unbounded_excluded_end (user e) :
    merge.user_key_in_range user core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Excluded e) =
      (do
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if true then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Inclusive end, unbounded start: key must be `<=` end. Dual-unfold. -/
theorem user_key_in_range_unbounded_included_end (user e) :
    merge.user_key_in_range user core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Included e) =
      (do
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if true then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Included start + included end: key `>=` s and key `<=` e. Dual-unfold. -/
theorem user_key_in_range_included_start_included_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Included e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Excluded start + excluded end: key `>` s and key `<` e. Dual-unfold. -/
theorem user_key_in_range_excluded_start_excluded_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Included start + excluded end: key `>=` s and key `<` e. Dual-unfold. -/
theorem user_key_in_range_included_start_excluded_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Excluded start + included end: key `>` s and key `<=` e. Dual-unfold. -/
theorem user_key_in_range_excluded_start_included_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Included e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Unbounded end never retires a stream. -/
theorem past_end_unbounded (k) :
    merge.past_end k core.ops.range.Bound.Unbounded = ok false := by
  unfold merge.past_end
  rfl

/-- Exclusive end retires at `>=` (half-open). Dual-unfold to slice `ge`. -/
theorem past_end_excluded (user e) :
    merge.past_end user (core.ops.range.Bound.Excluded e)
    = Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
        (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e := by
  unfold merge.past_end
  rfl

/-- Inclusive end retires at `>`. Dual-unfold to slice `gt`. -/
theorem past_end_included (user e) :
    merge.past_end user (core.ops.range.Bound.Included e)
    = Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
        (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e := by
  unfold merge.past_end
  rfl

/-- Hidden snapshot version is not emitted. -/
theorem iter_window_keep_hidden :
    merge.iter_window_keep false = ok false := by
  unfold merge.iter_window_keep
  rfl

/-- AS-IS dente: a hidden version still emits. -/
theorem iter_window_keep_as_is_dente :
    merge.iter_window_keep_as_is false = ok true := by
  unfold merge.iter_window_keep_as_is
  rfl

/-- Catalog entry: point put covers the exact start key (rustc `&[u8]`). -/
theorem write_op_covers_key_value_is_eq (start end1 user) :
    merge.write_op_covers_key key.ValueType.Value start end1 user
    = core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start user := by
  unfold merge.write_op_covers_key
  rfl

/-- Catalog entry: point delete covers the exact start key. -/
theorem write_op_covers_key_deletion_is_eq (start end1 user) :
    merge.write_op_covers_key key.ValueType.Deletion start end1 user
    = core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start user := by
  unfold merge.write_op_covers_key
  rfl

/-- Range delete covers via `range_tombstone_covers` (F30). Dual-unfold. -/
theorem write_op_covers_key_range (start end1 user) :
    merge.write_op_covers_key key.ValueType.RangeDeletion start end1 user
    = merge.range_tombstone_covers start end1 user := by
  unfold merge.write_op_covers_key
  rfl

/-- Catalog entry: rustc `&[u8]` cover is `>= start` then `< end` — not the u64 cartoon. -/
theorem range_tombstone_covers_is_ge_then_lt (start end1 user) :
    merge.range_tombstone_covers start end1 user =
      (do
        let b ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user start
        if b then
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user end1
        else ok false) := by
  unfold merge.range_tombstone_covers
  rfl

/-- AS-IS F30: rustc cover is start-key equality (misses interior). -/
theorem range_tombstone_covers_as_is_is_eq (start end1 user) :
    merge.range_tombstone_covers_as_is start end1 user
    = core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 user start := by
  unfold merge.range_tombstone_covers_as_is
  rfl

/-- AS-IS dente: point put never conflicts. -/
theorem write_op_covers_key_as_is_value (start end1 user) :
    merge.write_op_covers_key_as_is key.ValueType.Value start end1 user
    = ok false := by
  unfold merge.write_op_covers_key_as_is
  rfl

/-- AS-IS dente: point delete never conflicts. -/
theorem write_op_covers_key_as_is_deletion (start end1 user) :
    merge.write_op_covers_key_as_is key.ValueType.Deletion start end1 user
    = ok false := by
  unfold merge.write_op_covers_key_as_is
  rfl

/-- AS-IS dente: range only hits start. Dual-unfold. -/
theorem write_op_covers_key_as_is_range (start end1 user) :
    merge.write_op_covers_key_as_is key.ValueType.RangeDeletion start end1 user
    = merge.range_tombstone_covers_as_is start end1 user := by
  unfold merge.write_op_covers_key_as_is
  rfl

/-- Catalog entry: unbounded Bound copies as unbounded (rustc `Bound`). -/
theorem bound_to_owned_unbounded :
    merge.bound_to_owned core.ops.range.Bound.Unbounded
    = ok core.ops.range.Bound.Unbounded := by
  unfold merge.bound_to_owned
  rfl

/-- Catalog entry: unbounded Bound borrows as unbounded. -/
theorem bound_as_ref_unbounded :
    merge.bound_as_ref core.ops.range.Bound.Unbounded
    = ok core.ops.range.Bound.Unbounded := by
  unfold merge.bound_as_ref
  rfl

/-- Included Bound borrows via `Bytes::as_ref`. Dual-unfold. -/
theorem bound_as_ref_included (s) :
    merge.bound_as_ref (core.ops.range.Bound.Included s) =
      (do
        let s1 ← bytes.bytes.Bytes.Insts.CoreConvertAsRefSliceU8.as_ref s
        ok (core.ops.range.Bound.Included s1)) := by
  unfold merge.bound_as_ref
  rfl

/-- Excluded Bound borrows via `Bytes::as_ref`. Dual-unfold. -/
theorem bound_as_ref_excluded (s) :
    merge.bound_as_ref (core.ops.range.Bound.Excluded s) =
      (do
        let s1 ← bytes.bytes.Bytes.Insts.CoreConvertAsRefSliceU8.as_ref s
        ok (core.ops.range.Bound.Excluded s1)) := by
  unfold merge.bound_as_ref
  rfl

/-- Included Bound copies via `Bytes::copy_from_slice`. Dual-unfold. -/
theorem bound_to_owned_included (s) :
    merge.bound_to_owned (core.ops.range.Bound.Included s) =
      (do
        let b1 ← bytes.bytes.Bytes.copy_from_slice s
        ok (core.ops.range.Bound.Included b1)) := by
  unfold merge.bound_to_owned
  rfl

/-- Excluded Bound copies via `Bytes::copy_from_slice`. Dual-unfold. -/
theorem bound_to_owned_excluded (s) :
    merge.bound_to_owned (core.ops.range.Bound.Excluded s) =
      (do
        let b1 ← bytes.bytes.Bytes.copy_from_slice s
        ok (core.ops.range.Bound.Excluded b1)) := by
  unfold merge.bound_to_owned
  rfl

/-! ### RFC-0200 P1.1 — saída de merge alcançável (base: saída vazia) -/

/-- Saída produzida pelo merge: começa vazia e recebe um passo por
vez, EM ORDEM DE EMISSÃO (o passo recém-emissionado entra no fim) —
cada passo com o topo do heap newest-first (premissa estrutural por
passo). -/
inductive merge_output_reach : Nat → List MergeStep → Prop
  | empty : merge_output_reach 0 []
  | emit (k : Nat) (s : MergeStep) (out : List MergeStep) :
      merge_step_newest_first s →
      merge_output_reach k out →
      merge_output_reach (k + 1) (out ++ [s])

/-- Ponte produção→cadeia (RFC-0200 P1.1): uma saída alcançável em
ordem de emissão, lida de trás pra frente, É uma cadeia `merge_chain`
— o construtor cons da cadeia é a emissão mais recente. -/
theorem merge_output_reach_chain (k : Nat) (out : List MergeStep)
    (h : merge_output_reach k out) : merge_chain k out.reverse := by
  induction h with
  | empty => exact merge_chain.nil
  | emit k' s out' hnewest _ IH =>
      show merge_chain (k' + 1) (out' ++ [s]).reverse
      rw [List.reverse_append]
      exact merge_chain.cons s k' out'.reverse hnewest IH

/-- RFC-0200 P1.1 COROLÁRIO: toda saída que o merge produz a partir da
saída vazia (um passo por emissão, todo topo newest-first) contém
apenas versões genuinamente live nas que o filtro respondeu live —
composição da ponte com o corolário da cadeia (RFC-0198 P1.3);
nada é re-provado. -/
theorem merge_output_reach_preserves_inv_lsm :
    ∀ (k : Nat) (out : List MergeStep),
      merge_output_reach k out →
      ∀ s ∈ out,
        merge_step_answers_live s →
          s.kind = key.ValueType.Value ∧ s.range_hidden = false := by
  intro k out hreach s hs hlive
  have hchain := merge_output_reach_chain k out hreach
  have hmem : s ∈ out.reverse := List.mem_reverse.2 hs
  exact merge_chain_preserves_inv_lsm k out.reverse hchain s hmem hlive

/-! ### RFC-0200 P1.2 — ponte sift_step↔newest-first (camada tagged)

RE-ESCOPO DATADO 2026-09-11: o extract do sift não carrega estado de
heap (só os três bools) e o comparador é axioma
(`CoreCmpPartialOrdShared0B.lt`) — "o Swap restaura newest-first" não
é provável dos booleanos. A ponte honesta cobre o núcleo provável: a
decisão É a do kernel, o Stay é não-reparo (close registrado 0188) e
preserva a premissa por par, e em reparo o as-is fica onde o kernel
move. -/

/-- Um passo de sift com a decisão TOMADA PELO KERNEL sobre as três
entradas booleanas do extract (existe filho direito; direito <
esquerdo; melhor filho < buraco). -/
structure TaggedSift where
  r_exists : Bool
  r_lt_l : Bool
  best_lt_hole : Bool
  s : merge.SiftStep

/-- O campo `s` É a decisão do kernel sobre as entradas — não um valor
arbitrário. -/
def tagged_kernel_decision (t : TaggedSift) : Prop :=
  merge.sift_step t.r_exists t.r_lt_l t.best_lt_hole = ok t.s

/-- PONTE (Stay = não-reparo): a decisão do kernel é Stay exatamente
quando nenhum reparo é necessário — corolário DIRETO do close
REGISTRADO `merge_sift_step_repairs_iff` (RFC-0188 P0.2); nada é
re-provado. -/
theorem tagged_step_stays_iff_no_repair (t : TaggedSift)
    (ht : tagged_kernel_decision t) :
    (t.s = merge.SiftStep.Stay) ↔ (t.best_lt_hole = false) := by
  constructor
  · intro hstay
    unfold tagged_kernel_decision at ht
    rw [hstay] at ht
    exact (merge_sift_step_repairs_iff t.r_exists t.r_lt_l
      t.best_lt_hole).1 ht
  · intro hno
    have hk := (merge_sift_step_repairs_iff t.r_exists t.r_lt_l
      t.best_lt_hole).2 hno
    rw [ht] at hk
    simp only [Result.ok.injEq] at hk
    exact hk

/-- PONTE (par): a premissa estrutural da cadeia é LOCAL ao par de
idades — não lê `kind` nem `range_hidden`. -/
theorem merge_step_newest_first_congr (s s' : MergeStep)
    (hnew : s.newer = s'.newer) (hold : s.older = s'.older) :
    merge_step_newest_first s → merge_step_newest_first s' := by
  intro hprem
  unfold merge_step_newest_first at hprem ⊢
  rw [hnew, hold] at hprem
  exact hprem

/-- PONTE (Stay preserva): o kernel que fica não repara (iff
registrado) e a premissa estrutural carrega para o próximo passo de
MESMO par (o Stay não move ninguém) — composição das duas pontes. -/
theorem tagged_stay_preserves_newest_first (t : TaggedSift)
    (s s' : MergeStep) (ht : tagged_kernel_decision t)
    (hstay : t.s = merge.SiftStep.Stay)
    (hpair : s'.newer = s.newer ∧ s'.older = s.older)
    (hprem : merge_step_newest_first s) :
    t.best_lt_hole = false ∧ merge_step_newest_first s' :=
  ⟨(tagged_step_stays_iff_no_repair t ht).1 hstay,
    merge_step_newest_first_congr s s' hpair.1.symm hpair.2.symm hprem⟩

/-- O mutante as-is fica em TODO input de reparo — fato definicional
do dente (mesma forma de prova da divergência registrada). -/
theorem merge_sift_step_as_is_stays_on_repair :
    ∀ (r_exists r_lt_l : Bool),
      merge.sift_step_as_is r_exists r_lt_l true
        = ok merge.SiftStep.Stay := by
  intro r_exists r_lt_l
  unfold merge.sift_step_as_is
  cases r_exists <;> cases r_lt_l <;> simp

/-- PONTE (divergência em reparo): em todo input de reparo o kernel
NÃO devolve Stay (move o melhor filho para o buraco) enquanto o as-is
FICA — deixaria no topo o par que o kernel teria consertado. Cita o
close registrado e o fato definicional do as-is. -/
theorem tagged_repair_kernel_moves_as_is_stays (t : TaggedSift)
    (ht : tagged_kernel_decision t) (hrep : t.best_lt_hole = true) :
    t.s ≠ merge.SiftStep.Stay
    ∧ merge.sift_step_as_is t.r_exists t.r_lt_l t.best_lt_hole
        = ok merge.SiftStep.Stay := by
  constructor
  · intro hstay
    have hno := (tagged_step_stays_iff_no_repair t ht).1 hstay
    rw [hrep] at hno
    simp at hno
  · rw [hrep]
    exact merge_sift_step_as_is_stays_on_repair t.r_exists t.r_lt_l

/-- COROLÁRIO da ponte na cadeia: um passo Stay do kernel estende a
cadeia — o próximo passo de mesmo par continua newest-first (o cons é
a emissão mais recente, lendo a cabeça como o topo atual). -/
theorem tagged_stay_extends_chain (t : TaggedSift) (s s' : MergeStep)
    (k : Nat) (rest : List MergeStep) (ht : tagged_kernel_decision t)
    (hstay : t.s = merge.SiftStep.Stay)
    (hpair : s'.newer = s.newer ∧ s'.older = s.older)
    (hprem : merge_step_newest_first s)
    (hchain : merge_chain k (s :: rest)) :
    merge_chain (k + 1) (s' :: s :: rest) := by
  have hbridge := tagged_stay_preserves_newest_first t s s' ht hstay
    hpair hprem
  exact merge_chain.cons s' k (s :: rest) hbridge.2 hchain

/-- RFC-0213 P1.2 (storage cadence, atom `catalog:visible_at`): the
    merge get-filter answers live EXACTLY along the extracted route —
    a plain Value is live iff no covering range hides it; Deletion and
    RangeDeletion are never live, whatever the cover (fate forall over
    the extracted body, RFC-0170 P2.4). The AS-IS mutant answers live
    for every version (the lie the DST plant
    `visible_at_on_live_range_del_is_not_ok` refutes). -/
theorem visible_at_fate_iff :
    ∀ (kind : key.ValueType) (range_hidden : Bool) (v : Bool),
    (merge.visible_at kind range_hidden = ok v) ↔
      ((kind = key.ValueType.Value ∧ v = !range_hidden) ∨
       ((kind = key.ValueType.Deletion ∨ kind = key.ValueType.RangeDeletion)
          ∧ v = false)) := by
  intro kind range_hidden v
  unfold merge.visible_at
  cases kind <;> cases range_hidden <;> simp

/-- RFC-0218 P1.2 2/11 (átomo `catalog:write_op_range_end`, entrada
    `merge.write_op_range_end`): o fim do range é EXATAMENTE o despacho
    citado — Deletion e Value não têm fim (none); RangeDeletion carrega
    o próprio valor (some value). O AS-IS devolve sempre none (fim de
    range engolido — dente plantado). -/
theorem write_op_range_end_fate_iff :
    ∀ (kind : key.ValueType) (value : Slice U8)
      (r : Option (Slice U8)),
      (merge.write_op_range_end kind value = ok r) ↔
      ((kind = key.ValueType.Deletion ∧ r = none) ∨
       (kind = key.ValueType.Value ∧ r = none) ∨
       (kind = key.ValueType.RangeDeletion ∧ r = some value)) := by
  intro kind value r
  constructor
  · intro hval
    unfold merge.write_op_range_end at hval
    cases kind with
    | Deletion => injection hval with hv; exact Or.inl ⟨rfl, hv.symm⟩
    | Value => injection hval with hv; exact Or.inr (Or.inl ⟨rfl, hv.symm⟩)
    | RangeDeletion =>
      injection hval with hv
      exact Or.inr (Or.inr ⟨rfl, hv.symm⟩)
  · rintro (⟨hk, hv⟩ | ⟨hk, hv⟩ | ⟨hk, hv⟩)
    · subst hk; subst hv; rfl
    · subst hk; subst hv; rfl
    · subst hk; subst hv; rfl
