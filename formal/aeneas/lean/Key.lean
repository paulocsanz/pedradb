-- Theorems over Aeneas extract of key.rs (RFC-0150 P1).
import Aeneas
import KeyKernel
open Aeneas.Std Result
open pedra_aeneas_key_kernel

/-- Catalog entry: extracted `pack_sequence_and_type` is assert + shift + OR. -/
theorem pack_sequence_and_type_def (seq : U64) (kind : key.ValueType) :
    key.pack_sequence_and_type seq kind = (
      do
        let i ← key.MAX_SEQUENCE_NUMBER
        massert (seq <= i)
        let i1 ← seq <<< (8#i32)
        let i2 ← key.ValueType.as_u8 kind
        let i3 ← lift (core.convert.num.FromU64U8.from i2)
        ok (i1 ||| i3)
    ) := by
  unfold key.pack_sequence_and_type
  rfl

/-- AS-IS dente: seq=1 Deletion collides with seq=0 Value. -/
theorem pack_sequence_and_type_as_is_dente :
    key.pack_sequence_and_type_as_is (1#u64) key.ValueType.Deletion
      = key.pack_sequence_and_type_as_is (0#u64) key.ValueType.Value := by
  unfold key.pack_sequence_and_type_as_is
  unfold key.ValueType.as_u8
  rfl

/-- Catalog entry: trailer nibble 0 is Deletion (Rocks layout, not history wire). -/
theorem value_type_from_u8_deletion :
    key.ValueType.from_u8 0#u8 = ok (some key.ValueType.Deletion) := by
  unfold key.ValueType.from_u8
  rfl

/-- Catalog entry: trailer nibble 1 is Value. -/
theorem value_type_from_u8_value :
    key.ValueType.from_u8 1#u8 = ok (some key.ValueType.Value) := by
  unfold key.ValueType.from_u8
  rfl

/-- Catalog entry: trailer nibble 2 is RangeDeletion. -/
theorem value_type_from_u8_range_deletion :
    key.ValueType.from_u8 2#u8 = ok (some key.ValueType.RangeDeletion) := by
  unfold key.ValueType.from_u8
  rfl

/-- Catalog entry: unknown nibble is none. -/
theorem value_type_from_u8_unknown :
    key.ValueType.from_u8 3#u8 = ok none := by
  unfold key.ValueType.from_u8
  rfl

/-- Catalog entry: lookup probe is `new` at snapshot with Value (kValueTypeForSeek). Dual-unfold. -/
theorem internal_key_for_lookup_is_new_value
    {T0 : Type} (inst : core.convert.Into T0 bytes.bytes.Bytes)
    (user_key : T0) (snapshot : U64) :
    key.InternalKey.for_lookup inst user_key snapshot
    = key.InternalKey.new inst user_key snapshot key.ValueType.Value := by
  unfold key.InternalKey.for_lookup
  rfl

/-- Catalog entry: `new` is Into-bytes then the three fields. Dual-unfold. -/
theorem internal_key_new_into
    {T0 : Type} (inst : core.convert.Into T0 bytes.bytes.Bytes)
    (user_key : T0) (sequence : U64) (kind : key.ValueType) :
    key.InternalKey.new inst user_key sequence kind =
      (do
        let b ← inst.into user_key
        ok { user_key := b, sequence, kind }) := by
  unfold key.InternalKey.new
  rfl

/-- Catalog entry: sequence compare is reversed (`b.cmp(a)`) — newest first. Dual-unfold. -/
theorem ikey_seq_cmp_is_reverse (a b : U64) :
    key.ikey_seq_cmp a b = ok (core.cmp.impls.OrdU64.cmp b a) := by
  unfold key.ikey_seq_cmp
  rfl

/-- Catalog entry: `encode` is capacity + `encode_into` + `Bytes::from`. Dual-unfold. -/
theorem internal_key_encode_is_encode_into (self : key.InternalKey) :
    key.InternalKey.encode self =
      (do
        let i ← bytes.bytes.Bytes.len self.user_key
        let i1 ← i + 8#usize
        let buf := alloc.vec.Vec.with_capacity U8 i1
        let buf1 ← key.InternalKey.encode_into self buf
        bytes.bytes.Bytes.Insts.CoreConvertFromVecU8.from buf1) := by
  unfold key.InternalKey.encode
  rfl

/-- Catalog entry: Ord is user-key slice cmp first, then `ikey_seq_cmp`, then kind reverse. Dual-unfold. -/
theorem internal_key_cmp_user_key_then_seq
    (self other : key.InternalKey) :
    key.InternalKey.Insts.CoreCmpOrd.cmp self other =
      (do
        let s ←
          bytes.bytes.Bytes.Insts.CoreConvertAsRefSliceU8.as_ref self.user_key
        let s1 ←
          bytes.bytes.Bytes.Insts.CoreConvertAsRefSliceU8.as_ref other.user_key
        let o ← Slice.Insts.CoreCmpOrd.cmp core.cmp.OrdU8 s s1
        match o with
        | Ordering.lt => ok Ordering.lt
        | Ordering.eq =>
          let o1 ← key.ikey_seq_cmp self.sequence other.sequence
          match o1 with
          | Ordering.lt => ok Ordering.lt
          | Ordering.eq => key.ValueType.Insts.CoreCmpOrd.cmp other.kind self.kind
          | Ordering.gt => ok Ordering.gt
        | Ordering.gt => ok Ordering.gt) := by
  unfold key.InternalKey.Insts.CoreCmpOrd.cmp
  rfl
