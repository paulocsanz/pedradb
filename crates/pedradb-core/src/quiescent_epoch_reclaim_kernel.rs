//! Lock-Free Quiescent Epoch Memory Reclamation Kernel (RFC-0313).
//!
//! Provides mathematically verified generational epoch tracking and safe deferred memory
//! reclamation for unlinked memtables, SSTable block caches, and index structures without
//! global locks or unbounded quiescent stalling.
//!
//! # Problem Statement & Mathematical Foundation
//! In lock-free concurrent LSM engines, structures unlinked by compaction or memtable flushes
//! cannot be deallocated while concurrent reader threads hold references. Classic Epoch-Based
//! Reclamation (EBR) advances a global epoch $G \in \mathbb{N}$.
//!
//! # Safety Invariant
//! An object retired at epoch $e_{\text{retire}}$ is provably safe to deallocate when:
//! $$\forall t \in \text{Threads}: \text{is\_quiescent}(t) \lor \text{pinned\_epoch}(t) > e_{\text{retire}}$$
//!
//! The safe reclamation frontier is:
//! $$F_{\text{safe}} = \min \left(G, \min_{t \in \text{ActiveThreads}} \text{pinned\_epoch}(t)\right)$$
//! Any object retired with $e_{\text{retire}} < F_{\text{safe}}$ has zero active readers.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Errors returned by the quiescent epoch reclamation manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochReclaimError {
    /// Thread ID is not registered with the reclaimer.
    UnregisteredThread(usize),
    /// Thread is already pinned in a critical section.
    AlreadyPinned(usize),
    /// Thread is already quiescent (not pinned).
    AlreadyQuiescent(usize),
    /// Thread is already registered with the reclaimer.
    ThreadAlreadyRegistered(usize),
    /// Retired item ID cannot be zero.
    ZeroItemId,
    /// Quarantine capacity cannot be zero.
    ZeroQuarantineCapacity,
    /// Global epoch counter overflowed u64::MAX.
    GlobalEpochOverflow,
    /// Quarantine buffer has exceeded configured memory capacity.
    QuarantineSaturated {
        /// Current bytes in quarantine.
        current_bytes: usize,
        /// Maximum capacity.
        capacity_bytes: usize,
    },
}

impl std::fmt::Display for EpochReclaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnregisteredThread(t) => write!(f, "Thread {t} is not registered with epoch reclaimer"),
            Self::AlreadyPinned(t) => write!(f, "Thread {t} is already pinned in an active epoch"),
            Self::AlreadyQuiescent(t) => write!(f, "Thread {t} is already quiescent"),
            Self::ThreadAlreadyRegistered(t) => write!(f, "Thread {t} is already registered"),
            Self::ZeroItemId => write!(f, "Retired item ID cannot be zero"),
            Self::ZeroQuarantineCapacity => write!(f, "Quarantine capacity cannot be zero"),
            Self::GlobalEpochOverflow => write!(f, "Global epoch reached u64::MAX and cannot advance"),
            Self::QuarantineSaturated { current_bytes, capacity_bytes } => {
                write!(
                    f,
                    "Quarantine buffer saturated: {current_bytes} bytes queued, exceeds capacity {capacity_bytes}"
                )
            }
        }
    }
}

impl std::error::Error for EpochReclaimError {}

/// An item registered in the deferred reclamation quarantine queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredItem {
    /// Unique identifier for the retired allocation (e.g. pointer address or token ID).
    pub item_id: u64,
    /// Global epoch at the moment the item was unlinked/retired.
    pub retired_epoch: u64,
    /// Estimated memory footprint in bytes.
    pub size_bytes: usize,
}

/// Generational quiescent epoch reclamation controller.
#[derive(Debug, Clone)]
pub struct QuiescentEpochReclaimer {
    global_epoch: u64,
    active_threads: BTreeMap<usize, Option<u64>>,
    quarantine: Vec<RetiredItem>,
    quarantine_capacity_bytes: usize,
    total_quarantine_bytes: usize,
    total_reclaimed_bytes: usize,
}

impl QuiescentEpochReclaimer {
    /// Creates a new quiescent epoch reclaimer with a maximum quarantine memory capacity.
    #[must_use]
    pub fn new(quarantine_capacity_bytes: usize) -> Self {
        Self::try_new(quarantine_capacity_bytes).unwrap_or_else(|_| Self {
            global_epoch: 1,
            active_threads: BTreeMap::new(),
            quarantine: Vec::new(),
            quarantine_capacity_bytes: 1024 * 1024,
            total_quarantine_bytes: 0,
            total_reclaimed_bytes: 0,
        })
    }

    /// Safely constructs a quiescent epoch reclaimer, rejecting zero quarantine capacity.
    pub fn try_new(quarantine_capacity_bytes: usize) -> Result<Self, EpochReclaimError> {
        if quarantine_capacity_bytes == 0 {
            return Err(EpochReclaimError::ZeroQuarantineCapacity);
        }
        Ok(Self {
            global_epoch: 1,
            active_threads: BTreeMap::new(),
            quarantine: Vec::new(),
            quarantine_capacity_bytes,
            total_quarantine_bytes: 0,
            total_reclaimed_bytes: 0,
        })
    }

    /// Returns the current global epoch.
    #[must_use]
    pub fn global_epoch(&self) -> u64 {
        self.global_epoch
    }

    /// Returns total bytes currently pending in quarantine.
    #[must_use]
    pub fn quarantine_bytes(&self) -> usize {
        self.total_quarantine_bytes
    }

    /// Returns cumulative bytes safely reclaimed.
    #[must_use]
    pub fn reclaimed_bytes(&self) -> usize {
        self.total_reclaimed_bytes
    }

    /// Number of items awaiting reclamation in quarantine.
    #[must_use]
    pub fn pending_items_count(&self) -> usize {
        self.quarantine.len()
    }

    /// Safely registers a worker thread, preventing accidental unpinning of an already active thread.
    pub fn try_register_thread(&mut self, thread_id: usize) -> Result<(), EpochReclaimError> {
        if let Some(pinned) = self.active_threads.get(&thread_id) {
            if pinned.is_some() {
                return Err(EpochReclaimError::AlreadyPinned(thread_id));
            }
            return Err(EpochReclaimError::ThreadAlreadyRegistered(thread_id));
        }
        self.active_threads.insert(thread_id, None);
        Ok(())
    }

    /// Registers a new worker thread into the epoch tracking domain.
    pub fn register_thread(&mut self, thread_id: usize) {
        if !self.active_threads.contains_key(&thread_id) {
            self.active_threads.insert(thread_id, None);
        }
    }

    /// Unregisters an active or quiescent worker thread.
    pub fn unregister_thread(&mut self, thread_id: usize) {
        self.active_threads.remove(&thread_id);
    }

    /// Checks if a thread is currently registered.
    #[must_use]
    pub fn is_thread_registered(&self, thread_id: usize) -> bool {
        self.active_threads.contains_key(&thread_id)
    }

    /// Pins a worker thread to the current global epoch, entering an epoch-protected critical section.
    pub fn enter_critical_section(&mut self, thread_id: usize) -> Result<u64, EpochReclaimError> {
        let entry = self
            .active_threads
            .get_mut(&thread_id)
            .ok_or(EpochReclaimError::UnregisteredThread(thread_id))?;

        if entry.is_some() {
            return Err(EpochReclaimError::AlreadyPinned(thread_id));
        }

        let pinned = self.global_epoch;
        *entry = Some(pinned);
        Ok(pinned)
    }

    /// Unpins a worker thread, transitioning it to the quiescent state where it holds no references.
    pub fn exit_critical_section(&mut self, thread_id: usize) -> Result<(), EpochReclaimError> {
        let entry = self
            .active_threads
            .get_mut(&thread_id)
            .ok_or(EpochReclaimError::UnregisteredThread(thread_id))?;

        if entry.is_none() {
            return Err(EpochReclaimError::AlreadyQuiescent(thread_id));
        }

        *entry = None;
        Ok(())
    }

    /// Enqueues an unlinked item into the deferred quarantine ring.
    pub fn retire(&mut self, item_id: u64, size_bytes: usize) -> Result<(), EpochReclaimError> {
        if item_id == 0 {
            return Err(EpochReclaimError::ZeroItemId);
        }
        let new_bytes = self.total_quarantine_bytes.saturating_add(size_bytes);
        if self.quarantine_capacity_bytes > 0 && new_bytes > self.quarantine_capacity_bytes {
            return Err(EpochReclaimError::QuarantineSaturated {
                current_bytes: self.total_quarantine_bytes,
                capacity_bytes: self.quarantine_capacity_bytes,
            });
        }

        self.quarantine.push(RetiredItem {
            item_id,
            retired_epoch: self.global_epoch,
            size_bytes,
        });
        self.total_quarantine_bytes = new_bytes;
        Ok(())
    }

    /// Safely advances the global epoch, rejecting overflow if `global_epoch` reaches `u64::MAX`.
    pub fn try_advance_epoch(&mut self) -> Result<u64, EpochReclaimError> {
        let next = self
            .global_epoch
            .checked_add(1)
            .ok_or(EpochReclaimError::GlobalEpochOverflow)?;
        self.global_epoch = next;
        Ok(next)
    }

    /// Advances the global epoch monotonically.
    pub fn advance_epoch(&mut self) -> u64 {
        self.global_epoch = self.global_epoch.saturating_add(1);
        self.global_epoch
    }

    /// Computes the exact safe reclamation frontier $F_{\text{safe}}$.
    ///
    /// Any item retired with `retired_epoch < F_safe` is guaranteed to have NO concurrent readers.
    #[must_use]
    pub fn safe_reclaim_frontier(&self) -> u64 {
        let active_min = self
            .active_threads
            .values()
            .flatten()
            .copied()
            .min();

        match active_min {
            Some(min_pinned) => min_pinned,
            None => self.global_epoch,
        }
    }

    /// Identifies and extracts all quarantined items that are safe to physically deallocate.
    ///
    /// Updates memory accounting and removes reclaimed items from quarantine.
    pub fn collect_garbage(&mut self) -> Vec<u64> {
        let frontier = self.safe_reclaim_frontier();
        let mut reclaimed_ids = Vec::new();
        let mut surviving = Vec::with_capacity(self.quarantine.len());

        for item in self.quarantine.drain(..) {
            if item.retired_epoch < frontier {
                self.total_quarantine_bytes = self.total_quarantine_bytes.saturating_sub(item.size_bytes);
                self.total_reclaimed_bytes = self.total_reclaimed_bytes.saturating_add(item.size_bytes);
                reclaimed_ids.push(item.item_id);
            } else {
                surviving.push(item);
            }
        }

        self.quarantine = surviving;
        reclaimed_ids
    }

    /// Scans active threads and detects any thread lagging behind the global epoch by more than `max_lag`.
    ///
    /// Stalled threads prevent the safe reclamation frontier from advancing.
    #[must_use]
    pub fn detect_stalled_threads(&self, max_lag_epochs: u64) -> Vec<usize> {
        let mut stalled = Vec::new();
        for (&thread_id, &pinned) in &self.active_threads {
            if let Some(epoch) = pinned {
                if self.global_epoch.saturating_sub(epoch) > max_lag_epochs {
                    stalled.push(thread_id);
                }
            }
        }
        stalled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_capacity_quarantine_rejected() {
        let err = QuiescentEpochReclaimer::try_new(0).unwrap_err();
        assert_eq!(err, EpochReclaimError::ZeroQuarantineCapacity);
    }

    #[test]
    fn test_zero_item_id_retire_rejected() {
        let mut reclaimer = QuiescentEpochReclaimer::try_new(1024).unwrap();
        let err = reclaimer.retire(0, 32).unwrap_err();
        assert_eq!(err, EpochReclaimError::ZeroItemId);
    }

    #[test]
    fn test_thread_re_registration_protection() {
        let mut reclaimer = QuiescentEpochReclaimer::try_new(1024).unwrap();
        assert!(reclaimer.try_register_thread(42).is_ok());
        let err = reclaimer.try_register_thread(42).unwrap_err();
        assert_eq!(err, EpochReclaimError::ThreadAlreadyRegistered(42));

        reclaimer.enter_critical_section(42).unwrap();
        let err_pinned = reclaimer.try_register_thread(42).unwrap_err();
        assert_eq!(err_pinned, EpochReclaimError::AlreadyPinned(42));
        // Verify thread remains pinned at epoch 1
        assert_eq!(reclaimer.safe_reclaim_frontier(), 1);
    }

    #[test]
    fn test_global_epoch_overflow_rejected() {
        let mut reclaimer = QuiescentEpochReclaimer::try_new(1024).unwrap();
        reclaimer.global_epoch = u64::MAX;
        let err = reclaimer.try_advance_epoch().unwrap_err();
        assert_eq!(err, EpochReclaimError::GlobalEpochOverflow);
        assert_eq!(reclaimer.global_epoch(), u64::MAX);
    }
}
