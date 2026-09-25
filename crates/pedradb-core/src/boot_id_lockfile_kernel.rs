//! Boot-ID Stale Lockfile Reclaim and Crash Recovery Kernel (RFC-0285 Pilar 9).
//!
//! Provides deterministic identification and atomic recycling of stale LOCK files
//! left by node crashes, power failures, or SIGKILL without requiring operator manual deletion.
//!
//! Axiom:
//! A LockToken(boot_id, pid, created_epoch) is valid iff:
//! CurrentBootId == token.boot_id AND ProcessAlive(token.pid).
//! Any discrepancy categorizes the lock as Dead/Zombie and authorizes atomic reclaim.

#![forbid(unsafe_code)]

/// Lock acquisition or validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockfileError {
    /// Active, live process on the same machine currently holds the lock.
    ContendedByLiveProcess { pid: u32, boot_id: String },
    /// Lock payload corrupted or truncated.
    MalformedLockPayload,
    /// IO failure writing or replacing lockfile.
    IoFailure(String),
}

/// Structured lockfile token stored durably on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileToken {
    /// Linux kernel boot UUID (from /proc/sys/kernel/random/boot_id).
    pub boot_id: String,
    /// OS Process Identifier.
    pub pid: u32,
    /// UNIX timestamp when lock was acquired.
    pub created_at_secs: u64,
}

impl LockfileToken {
    /// Serializes token to on-disk byte representation.
    pub fn encode(&self) -> Vec<u8> {
        format!("{}:{}:{}\n", self.boot_id, self.pid, self.created_at_secs).into_bytes()
    }

    /// Deserializes token from on-disk bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, LockfileError> {
        let text = std::str::from_utf8(bytes).map_err(|_| LockfileError::MalformedLockPayload)?;
        let parts: Vec<&str> = text.trim().split(':').collect();
        if parts.len() != 3 {
            return Err(LockfileError::MalformedLockPayload);
        }

        let boot_id = parts[0].to_string();
        let pid = parts[1]
            .parse::<u32>()
            .map_err(|_| LockfileError::MalformedLockPayload)?;
        let created_at_secs = parts[2]
            .parse::<u64>()
            .map_err(|_| LockfileError::MalformedLockPayload)?;

        Ok(Self {
            boot_id,
            pid,
            created_at_secs,
        })
    }
}

/// Evaluator for lockfile validity across reboots and crashes.
pub struct BootIdLockCoordinator {
    current_boot_id: String,
    current_pid: u32,
}

impl BootIdLockCoordinator {
    /// Creates coordinator with current host boot identity and pid.
    pub fn new(current_boot_id: String, current_pid: u32) -> Self {
        Self {
            current_boot_id,
            current_pid,
        }
    }

    /// Evaluates existing lockfile contents against the host environment.
    ///
    /// `process_exists` is an injectable oracle answering whether a given PID is alive on this host.
    pub fn evaluate_lock(
        &self,
        existing_payload: &[u8],
        process_exists: impl Fn(u32) -> bool,
    ) -> Result<LockAction, LockfileError> {
        if existing_payload.is_empty() {
            return Ok(LockAction::AcquireFresh);
        }

        let token = LockfileToken::decode(existing_payload)?;

        // Case 1: Stale lock from previous machine boot
        if token.boot_id != self.current_boot_id {
            return Ok(LockAction::ReclaimStaleLock {
                reason: format!(
                    "Machine rebooted (lock boot_id: {}, current: {})",
                    token.boot_id, self.current_boot_id
                ),
                stale_token: token,
            });
        }

        // Case 2: Same boot, but process died (SIGKILL / power cycle)
        if !process_exists(token.pid) {
            return Ok(LockAction::ReclaimStaleLock {
                reason: format!("Holding process PID {} no longer exists", token.pid),
                stale_token: token,
            });
        }

        // Case 3: Same process already holds lock (idempotent re-entry)
        if token.pid == self.current_pid {
            return Ok(LockAction::AlreadyHeldBySelf);
        }

        // Case 4: Live process actively owns the lock
        Err(LockfileError::ContendedByLiveProcess {
            pid: token.pid,
            boot_id: token.boot_id,
        })
    }

    /// Creates fresh token for local acquisition.
    pub fn create_token(&self, now_secs: u64) -> LockfileToken {
        LockfileToken {
            boot_id: self.current_boot_id.clone(),
            pid: self.current_pid,
            created_at_secs: now_secs,
        }
    }
}

/// Action determined by lockfile evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockAction {
    /// No existing lock; acquire fresh.
    AcquireFresh,
    /// Lock was already acquired by current process.
    AlreadyHeldBySelf,
    /// Existing lock is stale and can be atomically overwritten.
    ReclaimStaleLock {
        reason: String,
        stale_token: LockfileToken,
    },
}
