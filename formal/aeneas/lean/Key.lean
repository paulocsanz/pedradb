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

/-- Catalog entry: rustc `as_u8` of Deletion is nibble 0 (inverse of `from_u8`). -/
theorem value_type_as_u8_deletion :
    key.ValueType.as_u8 key.ValueType.Deletion = ok 0#u8 := by
  unfold key.ValueType.as_u8
  rfl

/-- Catalog entry: trailer nibble 1 is Value. -/
theorem value_type_from_u8_value :
    key.ValueType.from_u8 1#u8 = ok (some key.ValueType.Value) := by
  unfold key.ValueType.from_u8
  rfl

/-- Catalog entry: rustc `as_u8` of Value is nibble 1 (inverse of `from_u8`). -/
theorem value_type_as_u8_value :
    key.ValueType.as_u8 key.ValueType.Value = ok 1#u8 := by
  unfold key.ValueType.as_u8
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

/-- Catalog entry: unpack is nibble `from_u8` then seq `>> 8`. Dual-unfold. -/
theorem unpack_sequence_and_type_is_from_u8 (packed : U64) :
    key.unpack_sequence_and_type packed =
      (do
        let i ← lift (packed &&& 255#u64)
        let type_byte ← lift (UScalar.cast .U8 i)
        let o ← key.ValueType.from_u8 type_byte
        let r ←
          core.option.Option.ok_or_else
            key.unpack_sequence_and_type.closure.Insts.CoreOpsFunctionFnOnceTupleCoreError
            o type_byte
        let cf ← core.result.Result.Insts.CoreOpsTry.branch r
        match cf with
        | core.ops.control_flow.ControlFlow.Continue val =>
          let sequence ← packed >>> 8#i32
          ok (core.result.Result.Ok (sequence, val))
        | core.ops.control_flow.ControlFlow.Break residual =>
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
            (U64 × key.ValueType) (core.convert.FromSame error.CoreError)
            residual) := by
  unfold key.unpack_sequence_and_type
  rfl

/-- Catalog entry: `encode_into` is extend user_key, pack trailer, extend BE bytes. Dual-unfold. -/
theorem internal_key_encode_into_is_extend_packed
    (self : key.InternalKey) (out : alloc.vec.Vec U8) :
    key.InternalKey.encode_into self out =
      (do
        let s ←
          bytes.bytes.Bytes.Insts.CoreOpsDerefDerefSliceU8.deref self.user_key
        let out1 ← alloc.vec.Vec.extend_from_slice core.clone.CloneU8 out s
        let packed ← key.pack_sequence_and_type self.sequence self.kind
        let a ← lift (core.num.U64.to_be_bytes packed)
        let s1 ← lift (Array.to_slice a)
        alloc.vec.Vec.extend_from_slice core.clone.CloneU8 out1 s1) := by
  unfold key.InternalKey.encode_into
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

/-- Catalog entry: `decode` is `len < 8` fail-closed, else split + `unpack_sequence_and_type`. Dual-unfold. -/
theorem internal_key_decode_is_len_then_unpack (encoded : Slice U8) :
    key.InternalKey.decode encoded =
      (do
        let i := Slice.len encoded
        if i < 8#usize
        then
          let args := Slice.len encoded
          let a ← core.fmt.rt.Argument.new_display Usize.Insts.CoreFmtDisplay args
          let a1 ←
            core.fmt.Arguments.new
              (Array.make 34#usize [
                24#u8, 105#u8, 110#u8, 116#u8, 101#u8, 114#u8, 110#u8, 97#u8, 108#u8,
                32#u8, 107#u8, 101#u8, 121#u8, 32#u8, 116#u8, 111#u8, 111#u8, 32#u8,
                115#u8, 104#u8, 111#u8, 114#u8, 116#u8, 58#u8, 32#u8, 192#u8, 6#u8,
                32#u8, 98#u8, 121#u8, 116#u8, 101#u8, 115#u8, 0#u8
                ]) (Array.make 1#usize [ a ])
          let s ← alloc.fmt.format a1
          let s1 ← core.hint.must_use s
          ok (core.result.Result.Err (error.CoreError.Internal s1))
        else
          let i1 := Slice.len encoded
          let split ← i1 - 8#usize
          let s ←
            core.slice.index.Slice.index
              (core.slice.index.SliceIndexRangeToUsizeSlice U8) encoded
              { «end» := split }
          let user_key ← bytes.bytes.Bytes.copy_from_slice s
          let trailer := Array.repeat 8#usize 0#u8
          let (s1, to_slice_mut_back) ← lift (Array.to_slice_mut trailer)
          let s2 ←
            core.slice.index.Slice.index
              (core.slice.index.SliceIndexRangeFromUsizeSlice U8) encoded
              { start := split }
          let s3 ← core.slice.Slice.copy_from_slice core.marker.CopyU8 s1 s2
          let trailer1 := to_slice_mut_back s3
          let packed ← lift (core.num.U64.from_be_bytes trailer1)
          let r ← key.unpack_sequence_and_type packed
          let cf ← core.result.Result.Insts.CoreOpsTry.branch r
          match cf with
          | core.ops.control_flow.ControlFlow.Continue val =>
            let (sequence, kind) := val
            ok (core.result.Result.Ok { user_key, sequence, kind })
          | core.ops.control_flow.ControlFlow.Break residual =>
            core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
              key.InternalKey (core.convert.FromSame error.CoreError) residual) := by
  unfold key.InternalKey.decode
  rfl

/-- Catalog entry: PartialEq is `cmp == Equal`. Dual-unfold. -/
theorem internal_key_eq_is_cmp_equal
    (self other : key.InternalKey) :
    key.InternalKey.Insts.CoreCmpPartialEqInternalKey.eq self other =
      (do
        let o ← key.InternalKey.Insts.CoreCmpOrd.cmp self other
        core.cmp.Ordering.Insts.CoreCmpPartialEqOrdering.eq o Ordering.eq) := by
  unfold key.InternalKey.Insts.CoreCmpPartialEqInternalKey.eq
  rfl

/-- Catalog entry: PartialOrd is `Some(cmp)`. Dual-unfold. -/
theorem internal_key_partial_cmp_is_some_cmp
    (self other : key.InternalKey) :
    key.InternalKey.Insts.CoreCmpPartialOrdInternalKey.partial_cmp self other =
      (do
        let o ← key.InternalKey.Insts.CoreCmpOrd.cmp self other
        ok (some o)) := by
  unfold key.InternalKey.Insts.CoreCmpPartialOrdInternalKey.partial_cmp
  rfl
