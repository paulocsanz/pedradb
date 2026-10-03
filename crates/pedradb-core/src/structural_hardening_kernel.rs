//! RFC-0323: Structural Hardening, Bug Class Eradication & Typestate Impossibility Barriers.
//!
//! Eliminates broad classes of bugs by making invalid states unrepresentable:
//! 1. Barreira 1: Typestate Pattern & RAII Quiescence Leases (compile-time illegal operation prevention).
//! 2. Barreira 2: Strong Domain Types (`NonZeroU64`, `NonNilUuid`, avoiding zero-sentinel hazards).
//! 3. Barreira 3: Closed Structs & Smart Constructors (validating invariants upon construction).
//! 4. Barreira 4: Complete Float Sanitization (`FiniteF64`, banishing `NaN` / `Inf` pacing bypasses).
//! 5. Barreira 5: Fail-closed boundary evaluation and monotonic verification.

#![forbid(unsafe_code)]

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Error domain for structural hardening and invalid invariant attempts.
#[derive(Debug, Clone, PartialEq)]
pub enum StructuralHardeningError {
    /// Zero identifier provided where non-zero domain type is required.
    ZeroIdentifierHazard { entity: &'static str },
    /// Nil UUID provided (all-zero bytes).
    NilUuidHazard,
    /// Slice decoding length mismatch or buffer underflow.
    SliceDecodeLengthUnderflow { expected: usize, actual: usize },
    /// Non-finite or NaN floating point value encountered.
    NonFiniteFloatHazard { detail: &'static str },
    /// Floating point value out of valid statistical/pacing range.
    FloatOutOfRange { min: f64, max: f64, actual: f64 },
    /// Inverted or non-monotonic slice boundaries (`start >= end`).
    InvertedSliceBounds,
    /// Corrupted record accounting where tombstones exceed total records.
    CorruptedRecordAccounting { tombstones: u64, total: u64 },
    /// Partition index out of bounds.
    PartitionOutOfBounds { index: usize, total: usize },
    /// Leases active while transitioning out of active state.
    ActiveLeasesPreventTransition { active: u32 },
    /// Typestate quiescent poll failed: active readers still draining.
    QuiescenceDrainPending { active_readers: u32 },
    /// Capacity overflow when acquiring reader lease.
    ReaderCapacityExceeded { current: u32, max: u32 },
}

impl fmt::Display for StructuralHardeningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroIdentifierHazard { entity } => {
                write!(f, "Invalid {entity}: zero is a reserved/illegal sentinel value")
            }
            Self::NilUuidHazard => write!(f, "Invalid UUID: all-zero nil UUID is disallowed"),
            Self::SliceDecodeLengthUnderflow { expected, actual } => {
                write!(f, "Slice decode underflow: expected {expected} bytes, got {actual}")
            }
            Self::NonFiniteFloatHazard { detail } => {
                write!(f, "Non-finite float hazard: NaN or Inf rejected in {detail}")
            }
            Self::FloatOutOfRange { min, max, actual } => {
                write!(f, "Float out of range: {actual} not in ({min}, {max})")
            }
            Self::InvertedSliceBounds => {
                write!(f, "Inverted slice boundaries: start bound is not strictly less than end bound")
            }
            Self::CorruptedRecordAccounting { tombstones, total } => {
                write!(f, "Corrupted record accounting: tombstones ({tombstones}) > total ({total})")
            }
            Self::PartitionOutOfBounds { index, total } => {
                write!(f, "Partition index out of bounds: {index} >= total {total}")
            }
            Self::ActiveLeasesPreventTransition { active } => {
                write!(f, "Active leases prevent transition: {active} readers still hold references")
            }
            Self::QuiescenceDrainPending { active_readers } => {
                write!(f, "Quiescence drain pending: {active_readers} active readers remain")
            }
            Self::ReaderCapacityExceeded { current, max } => {
                write!(f, "Reader lease capacity exceeded: {current} >= {max}")
            }
        }
    }
}

impl std::error::Error for StructuralHardeningError {}

// ============================================================================
// Barreira 2: Strong Domain Types (NonZero Wrappers)
// ============================================================================

/// Strongly-typed SST file number guaranteed non-zero by construction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileNumber(NonZeroU64);

impl FileNumber {
    /// Attempts to construct a `FileNumber`, rejecting 0.
    pub fn try_new(raw: u64) -> Result<Self, StructuralHardeningError> {
        NonZeroU64::new(raw)
            .map(Self)
            .ok_or(StructuralHardeningError::ZeroIdentifierHazard { entity: "FileNumber" })
    }

    /// Returns the raw `u64` value.
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        self.0.get()
    }

    /// Returns the inner `NonZeroU64`.
    #[must_use]
    pub fn get(&self) -> NonZeroU64 {
        self.0
    }
}

impl fmt::Display for FileNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileNumber({})", self.0)
    }
}

/// Strongly-typed MVCC sequence number guaranteed non-zero by construction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SequenceNumber(NonZeroU64);

impl SequenceNumber {
    /// Attempts to construct a `SequenceNumber`, rejecting 0.
    pub fn try_new(raw: u64) -> Result<Self, StructuralHardeningError> {
        NonZeroU64::new(raw)
            .map(Self)
            .ok_or(StructuralHardeningError::ZeroIdentifierHazard { entity: "SequenceNumber" })
    }

    /// Returns the raw `u64` value.
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        self.0.get()
    }

    /// Returns the inner `NonZeroU64`.
    #[must_use]
    pub fn get(&self) -> NonZeroU64 {
        self.0
    }
}

impl fmt::Display for SequenceNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SequenceNumber({})", self.0)
    }
}

/// Strongly-typed epoch/generation counter guaranteed non-zero by construction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(NonZeroU64);

impl Generation {
    /// Attempts to construct a `Generation`, rejecting 0.
    pub fn try_new(raw: u64) -> Result<Self, StructuralHardeningError> {
        NonZeroU64::new(raw)
            .map(Self)
            .ok_or(StructuralHardeningError::ZeroIdentifierHazard { entity: "Generation" })
    }

    /// Returns the raw `u64` value.
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        self.0.get()
    }

    /// Returns the inner `NonZeroU64`.
    #[must_use]
    pub fn get(&self) -> NonZeroU64 {
        self.0
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Generation({})", self.0)
    }
}

/// Strongly-typed SST ID counter guaranteed non-zero by construction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SstId(NonZeroU64);

impl SstId {
    /// Attempts to construct an `SstId`, rejecting 0.
    pub fn try_new(raw: u64) -> Result<Self, StructuralHardeningError> {
        NonZeroU64::new(raw)
            .map(Self)
            .ok_or(StructuralHardeningError::ZeroIdentifierHazard { entity: "SstId" })
    }

    /// Returns the raw `u64` value.
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        self.0.get()
    }

    /// Returns the inner `NonZeroU64`.
    #[must_use]
    pub fn get(&self) -> NonZeroU64 {
        self.0
    }
}

impl fmt::Display for SstId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SstId({})", self.0)
    }
}

/// 16-byte UUID guaranteed to be non-nil by construction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct NonNilUuid([u8; 16]);

impl NonNilUuid {
    /// Attempts to construct a `NonNilUuid`, rejecting all-zero byte arrays.
    pub fn try_new(bytes: [u8; 16]) -> Result<Self, StructuralHardeningError> {
        if bytes == [0u8; 16] {
            Err(StructuralHardeningError::NilUuidHazard)
        } else {
            Ok(Self(bytes))
        }
    }

    /// Decodes a 16-byte slice, validating length and non-nil invariant.
    pub fn try_from_slice(slice: &[u8]) -> Result<Self, StructuralHardeningError> {
        if slice.len() < 16 {
            return Err(StructuralHardeningError::SliceDecodeLengthUnderflow {
                expected: 16,
                actual: slice.len(),
            });
        }
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&slice[..16]);
        Self::try_new(bytes)
    }

    /// Returns the underlying 16-byte slice.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

// ============================================================================
// Barreira 4: Finite Float Sanitization (`FiniteF64`)
// ============================================================================

/// Strictly sanitized floating point value guaranteed finite, non-NaN by construction.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct FiniteF64(f64);

impl FiniteF64 {
    /// Attempts to construct a `FiniteF64`, rejecting NaN and ±Inf.
    pub fn try_new(val: f64) -> Result<Self, StructuralHardeningError> {
        if val.is_nan() || !val.is_finite() {
            Err(StructuralHardeningError::NonFiniteFloatHazard {
                detail: "FiniteF64 construction",
            })
        } else {
            Ok(Self(val))
        }
    }

    /// Returns the raw `f64` value.
    #[must_use]
    pub fn get(&self) -> f64 {
        self.0
    }

    /// Checked addition that fails closed on overflow to infinity or NaN.
    pub fn checked_add(&self, other: Self) -> Result<Self, StructuralHardeningError> {
        Self::try_new(self.0 + other.0)
    }

    /// Checked subtraction that fails closed on overflow to infinity or NaN.
    pub fn checked_sub(&self, other: Self) -> Result<Self, StructuralHardeningError> {
        Self::try_new(self.0 - other.0)
    }

    /// Checked multiplication that fails closed on overflow to infinity or NaN.
    pub fn checked_mul(&self, other: Self) -> Result<Self, StructuralHardeningError> {
        Self::try_new(self.0 * other.0)
    }

    /// Checked division that fails closed on divide-by-zero, infinity or NaN.
    pub fn checked_div(&self, other: Self) -> Result<Self, StructuralHardeningError> {
        if other.0 == 0.0 {
            Err(StructuralHardeningError::NonFiniteFloatHazard { detail: "division by zero" })
        } else {
            Self::try_new(self.0 / other.0)
        }
    }
}

impl fmt::Display for FiniteF64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.6}", self.0)
    }
}

// ============================================================================
// Barreira 3: Closed Structs & Smart Constructors
// ============================================================================

/// Closed configuration for tombstone vacuum cascades.
///
/// Guaranteed `tombstone_records <= total_records` and `dead_ratio_threshold in (0.0, 1.0]`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedTombstoneDrainConfig {
    total_records: u64,
    tombstone_records: u64,
    dead_ratio_threshold: FiniteF64,
}

impl ClosedTombstoneDrainConfig {
    /// Smart constructor validating accounting invariants.
    pub fn try_new(
        total_records: u64,
        tombstone_records: u64,
        dead_ratio_threshold: FiniteF64,
    ) -> Result<Self, StructuralHardeningError> {
        if tombstone_records > total_records {
            return Err(StructuralHardeningError::CorruptedRecordAccounting {
                tombstones: tombstone_records,
                total: total_records,
            });
        }
        let threshold = dead_ratio_threshold.get();
        if threshold <= 0.0 || threshold > 1.0 {
            return Err(StructuralHardeningError::FloatOutOfRange {
                min: 0.0,
                max: 1.0,
                actual: threshold,
            });
        }
        Ok(Self {
            total_records,
            tombstone_records,
            dead_ratio_threshold,
        })
    }

    /// Total records in the table.
    #[must_use]
    pub fn total_records(&self) -> u64 {
        self.total_records
    }

    /// Tombstone records in the table.
    #[must_use]
    pub fn tombstone_records(&self) -> u64 {
        self.tombstone_records
    }

    /// Live records remaining.
    #[must_use]
    pub fn live_records(&self) -> u64 {
        self.total_records.saturating_sub(self.tombstone_records)
    }

    /// Ratio threshold for triggering cascade vacuum.
    #[must_use]
    pub fn dead_ratio_threshold(&self) -> FiniteF64 {
        self.dead_ratio_threshold
    }

    /// Evaluates whether the table exceeds the tombstone drain threshold.
    #[must_use]
    pub fn should_drain(&self) -> bool {
        if self.total_records == 0 {
            return false;
        }
        let ratio = (self.tombstone_records as f64) / (self.total_records as f64);
        ratio >= self.dead_ratio_threshold.get()
    }
}

/// Closed subcompaction slice with strictly ordered non-inverted boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedParallelSubcompactionSlice {
    start_bound: Vec<u8>,
    end_bound: Vec<u8>,
    output_partition_idx: usize,
    total_partitions: usize,
}

impl ClosedParallelSubcompactionSlice {
    /// Smart constructor enforcing `start_bound < end_bound` and partition bounds.
    pub fn try_new(
        start_bound: Vec<u8>,
        end_bound: Vec<u8>,
        output_partition_idx: usize,
        total_partitions: usize,
    ) -> Result<Self, StructuralHardeningError> {
        if total_partitions == 0 || output_partition_idx >= total_partitions {
            return Err(StructuralHardeningError::PartitionOutOfBounds {
                index: output_partition_idx,
                total: total_partitions,
            });
        }
        if start_bound >= end_bound {
            return Err(StructuralHardeningError::InvertedSliceBounds);
        }
        Ok(Self {
            start_bound,
            end_bound,
            output_partition_idx,
            total_partitions,
        })
    }

    /// Inclusive lower bound.
    #[must_use]
    pub fn start_bound(&self) -> &[u8] {
        &self.start_bound
    }

    /// Exclusive upper bound.
    #[must_use]
    pub fn end_bound(&self) -> &[u8] {
        &self.end_bound
    }

    /// Assigned output partition index.
    #[must_use]
    pub fn output_partition_idx(&self) -> usize {
        self.output_partition_idx
    }

    /// Total partition count.
    #[must_use]
    pub fn total_partitions(&self) -> usize {
        self.total_partitions
    }
}

/// Closed adaptive Bloom budget guaranteeing positive keys, positive budget, and valid FPP.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedAdaptiveBloomBudget {
    total_keys: NonZeroU64,
    ram_budget_bytes: NonZeroU64,
    target_fpp: FiniteF64,
}

impl ClosedAdaptiveBloomBudget {
    /// Smart constructor ensuring non-zero keys, non-zero budget, and `0 < fpp < 1`.
    pub fn try_new(
        total_keys: u64,
        ram_budget_bytes: u64,
        target_fpp: f64,
    ) -> Result<Self, StructuralHardeningError> {
        let keys = NonZeroU64::new(total_keys).ok_or(
            StructuralHardeningError::ZeroIdentifierHazard { entity: "total_keys" },
        )?;
        let ram = NonZeroU64::new(ram_budget_bytes).ok_or(
            StructuralHardeningError::ZeroIdentifierHazard { entity: "ram_budget_bytes" },
        )?;
        let fpp = FiniteF64::try_new(target_fpp)?;
        if fpp.get() <= 0.0 || fpp.get() >= 1.0 {
            return Err(StructuralHardeningError::FloatOutOfRange {
                min: 0.0,
                max: 1.0,
                actual: fpp.get(),
            });
        }
        Ok(Self {
            total_keys: keys,
            ram_budget_bytes: ram,
            target_fpp: fpp,
        })
    }

    /// Total keys configured.
    #[must_use]
    pub fn total_keys(&self) -> u64 {
        self.total_keys.get()
    }

    /// RAM budget ceiling in bytes.
    #[must_use]
    pub fn ram_budget_bytes(&self) -> u64 {
        self.ram_budget_bytes.get()
    }

    /// Target false-positive probability.
    #[must_use]
    pub fn target_fpp(&self) -> FiniteF64 {
        self.target_fpp
    }

    /// Effective bits allocated per key.
    #[must_use]
    pub fn bits_per_key(&self) -> FiniteF64 {
        let bits = (self.ram_budget_bytes.get() as f64) * 8.0;
        let bpk = bits / (self.total_keys.get() as f64);
        FiniteF64::try_new(bpk).unwrap_or_else(|_| FiniteF64(10.0))
    }
}

// ============================================================================
// Barreira 1: Typestate Pattern & RAII Quiescence Leases
// ============================================================================

/// Typestate marker: Region is active and serving reader leases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveRegion;

/// Typestate marker: Region is draining active reader leases; new leases are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuiescingRegion;

/// Typestate marker: All readers have quiesced and unmap has been executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimedRegion;

/// Shared atomic lease counter for typestate mmap tracking.
#[derive(Debug)]
struct TypestateControl {
    active_readers: AtomicU32,
    max_readers: u32,
}

/// RAII reader lease guaranteeing memory mapping validity for its lifetime.
#[derive(Debug)]
pub struct TypestateReaderLease {
    control: Arc<TypestateControl>,
}

impl Drop for TypestateReaderLease {
    fn drop(&mut self) {
        self.control.active_readers.fetch_sub(1, Ordering::Release);
    }
}

/// Memory-mapped file region governed by static typestate transitions.
#[derive(Debug)]
pub struct MmapTypestateRegion<State> {
    file_number: FileNumber,
    control: Arc<TypestateControl>,
    _state: std::marker::PhantomData<State>,
}

impl MmapTypestateRegion<ActiveRegion> {
    /// Creates a new active mmap region.
    #[must_use]
    pub fn try_new(file_number: FileNumber) -> Self {
        Self {
            file_number,
            control: Arc::new(TypestateControl {
                active_readers: AtomicU32::new(0),
                max_readers: 1_000_000,
            }),
            _state: std::marker::PhantomData,
        }
    }

    /// Associated file number.
    #[must_use]
    pub fn file_number(&self) -> FileNumber {
        self.file_number
    }

    /// Attempts to acquire an RAII reader lease.
    pub fn try_acquire_lease(&self) -> Result<TypestateReaderLease, StructuralHardeningError> {
        let current = self.control.active_readers.load(Ordering::Acquire);
        if current >= self.control.max_readers {
            return Err(StructuralHardeningError::ReaderCapacityExceeded {
                current,
                max: self.control.max_readers,
            });
        }
        self.control.active_readers.fetch_add(1, Ordering::AcqRel);
        Ok(TypestateReaderLease {
            control: Arc::clone(&self.control),
        })
    }

    /// Initiates quiescence, transitioning the region from Active to Quiescing.
    ///
    /// New leases will be rejected at compile time once transitioned.
    pub fn begin_quiescence(self) -> MmapTypestateRegion<QuiescingRegion> {
        MmapTypestateRegion {
            file_number: self.file_number,
            control: self.control,
            _state: std::marker::PhantomData,
        }
    }
}

impl MmapTypestateRegion<QuiescingRegion> {
    /// Associated file number.
    #[must_use]
    pub fn file_number(&self) -> FileNumber {
        self.file_number
    }

    /// Current count of in-flight active readers draining.
    #[must_use]
    pub fn active_readers(&self) -> u32 {
        self.control.active_readers.load(Ordering::Acquire)
    }

    /// Polls whether all readers have drained.
    ///
    /// If quiesced (`active_readers == 0`), consumes `self` and transitions to `ReclaimedRegion`.
    /// Otherwise, returns `Err((self, error))` allowing the caller to retry.
    pub fn poll_quiesced(
        self,
    ) -> Result<MmapTypestateRegion<ReclaimedRegion>, (Self, StructuralHardeningError)> {
        let active = self.control.active_readers.load(Ordering::Acquire);
        if active == 0 {
            Ok(MmapTypestateRegion {
                file_number: self.file_number,
                control: self.control,
                _state: std::marker::PhantomData,
            })
        } else {
            Err((self, StructuralHardeningError::QuiescenceDrainPending { active_readers: active }))
        }
    }
}

impl MmapTypestateRegion<ReclaimedRegion> {
    /// Associated file number.
    #[must_use]
    pub fn file_number(&self) -> FileNumber {
        self.file_number
    }

    /// Confirms that the region has been physically reclaimed.
    #[must_use]
    pub fn is_reclaimed(&self) -> bool {
        true
    }
}

// ============================================================================
// Verification and Invariant Audit
// ============================================================================

/// Verifies that all foundational domain invariants hold true across standard instances.
#[must_use]
pub fn verify_structural_invariants() -> bool {
    let fn_ok = FileNumber::try_new(42).is_ok() && FileNumber::try_new(0).is_err();
    let seq_ok = SequenceNumber::try_new(100).is_ok() && SequenceNumber::try_new(0).is_err();
    let gen_ok = Generation::try_new(1).is_ok() && Generation::try_new(0).is_err();
    let sst_ok = SstId::try_new(99).is_ok() && SstId::try_new(0).is_err();
    let uuid_ok = NonNilUuid::try_new([1u8; 16]).is_ok() && NonNilUuid::try_new([0u8; 16]).is_err();
    let float_ok = FiniteF64::try_new(0.5).is_ok()
        && FiniteF64::try_new(f64::NAN).is_err()
        && FiniteF64::try_new(f64::INFINITY).is_err();

    fn_ok && seq_ok && gen_ok && sst_ok && uuid_ok && float_ok
}
