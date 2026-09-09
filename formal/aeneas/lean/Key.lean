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
