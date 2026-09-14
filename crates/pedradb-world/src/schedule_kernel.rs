//! Seed → finite action list for a World run (buggify-lite).

use pedradb_core::{Rng, SeedRng};

/// One atomic step the World applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Drive store elections / heartbeats this many logical ticks.
    StoreTicks(u32),
    /// Advance store logical time by `n` (alias of StoreTicks for P1.5 clarity).
    ClockAdvance(u32),
    /// Client put (key, value).
    Put {
        /// Key suffix byte.
        key_tag: u8,
        /// Value suffix byte.
        val_tag: u8,
    },
    /// Client get (local applied on a node).
    Get {
        /// Key suffix byte.
        key_tag: u8,
        /// Node id (1-based) to read.
        node: u64,
    },
    /// Strong get via live leader.
    GetStrong {
        /// Key suffix byte.
        key_tag: u8,
    },
    /// DCS create-if-absent under `m/` prefix.
    DcsCreate {
        /// Key suffix under meta prefix.
        key_tag: u8,
        /// Value suffix.
        val_tag: u8,
        /// TTL ms (`0` = immortal). Absolute deadline baked at propose.
        ttl_ms: u64,
    },
    /// Advance cluster lease clock only (no raft tick).
    AdvanceNowMs {
        /// Milliseconds to add to `StoreCluster::now_ms`.
        ms: u64,
    },
    /// DCS compare-and-swap (expected_rev from prior create when possible).
    DcsCas {
        /// Key suffix under meta prefix.
        key_tag: u8,
        /// Value suffix.
        val_tag: u8,
        /// Expected revision (0 = will often CasFail; schedule uses small values).
        expected_rev: u64,
    },
    /// Partition node offline (store `set_participating(false)`).
    Partition {
        /// Peer id (1-based).
        node: u64,
    },
    /// Heal membership partition.
    Heal {
        /// Peer id (1-based).
        node: u64,
    },
    /// Kill peer: remove from Raft membership (P2.2).
    RemoveMember {
        /// Peer id (1-based).
        node: u64,
    },
    /// Re-admit peer (install-snapshot catch-up path).
    AddMember {
        /// Peer id (1-based).
        node: u64,
    },
    /// Arm per-peer disk fault (`FailingEnv::arm`).
    DiskArm {
        /// Peer id (1-based).
        node: u64,
        /// Ops that still succeed before the fault fires.
        after_ops: u64,
        /// One-shot vs permanent until disarm.
        transient: bool,
    },
    /// Clear disk fault on a peer.
    DiskDisarm {
        /// Peer id (1-based).
        node: u64,
    },
    /// Inject N opaque net messages between random peers.
    NetSpray {
        /// How many envelopes to inject.
        count: u8,
    },
    /// Advance net clock.
    NetTick(u32),
    /// Drain ready net deliveries.
    NetDrain,
    /// G2 canary (RFC-0051 P1.2): propose an atomic index-row batch, make
    /// the outcome unknown (partition the leader mid-flight), resolve
    /// NotCommitted-after-majority, heal, retry the same batch, then check
    /// all-or-nothing (never `row_half_indexed`).
    CommitUnknown {
        /// Key suffix byte (row + both secondary index keys derive from it).
        key_tag: u8,
    },
    /// Flush every node's engine (WAL → SST) so a subsequent put is tail-only.
    FlushAll,
    /// Log-carried joint membership remove (RFC-0063 P0).
    JointRemove {
        /// Peer id (1-based).
        node: u64,
    },
    /// Log-carried joint membership add (RFC-0064 P0).
    JointAdd {
        /// Peer id (1-based).
        node: u64,
    },
    /// Process-style crash: drop unsynced Env bytes, drop engines without
    /// `Db::close` (close would flush — clean shutdown), reopen from durable.
    CrashReopen,
    /// RFC-0060 P1.2: XOR `n_bits` of an already-durable page on `node`
    /// (SST / WAL / vlog). Subsequent reads must fail-closed or return the
    /// correct value — never silent-wrong.
    BitFlip {
        /// Peer id (1-based).
        node: u64,
        /// How many bits to invert (1 is enough to trip CRC).
        n_bits: u32,
        /// When false, select the same file/offset but do not write — World
        /// mutant of the silent-wrong oracle (RFC-0060 P1.2).
        apply: bool,
    },
    /// RFC-0068: plant a committed C-old,new joint with apply lag and no
    /// leave (crash window). C-old majority must not elect.
    PlantCommittedJoint {
        /// Peer id to add in the planted joint (1-based; must not be a voter).
        node: u64,
    },
    /// RFC-0067 P1.1: after World pins Queued, attempt `RpcMode::Direct`.
    /// Direct after pin is silent_wrong (Net drop/reorder become no-ops).
    AttemptDirectRpc,
}

/// Expand `seed` into a fixed-length schedule (deterministic).
#[must_use]
pub fn schedule_from_seed(seed: u64, n_nodes: u64, steps: usize) -> Vec<Action> {
    let rng = SeedRng::new(seed ^ 0x5C4ED);
    let mut out = Vec::with_capacity(steps + 24);
    // Elect first via logical clock.
    out.push(Action::ClockAdvance(40));
    for _ in 0..steps {
        let op = rng.gen_range(12);
        match op {
            0 | 1 => {
                out.push(Action::Put {
                    key_tag: (rng.gen_range(16) as u8).saturating_add(1),
                    val_tag: rng.gen_range(256) as u8,
                });
                out.push(Action::ClockAdvance(2));
            }
            2 => {
                out.push(Action::Get {
                    key_tag: (rng.gen_range(16) as u8).saturating_add(1),
                    node: 1 + rng.gen_range(n_nodes.max(1)),
                });
            }
            3 => {
                out.push(Action::GetStrong {
                    key_tag: (rng.gen_range(16) as u8).saturating_add(1),
                });
            }
            4 => {
                let ttl = if rng.gen_range(3) == 0 {
                    50 + rng.gen_range(200)
                } else {
                    0
                };
                out.push(Action::DcsCreate {
                    key_tag: (rng.gen_range(8) as u8).saturating_add(1),
                    val_tag: rng.gen_range(256) as u8,
                    ttl_ms: ttl,
                });
                if ttl > 0 {
                    out.push(Action::AdvanceNowMs { ms: ttl + 1 });
                    out.push(Action::DcsCreate {
                        key_tag: (rng.gen_range(8) as u8).saturating_add(1),
                        val_tag: rng.gen_range(256) as u8,
                        ttl_ms: 0,
                    });
                }
                out.push(Action::ClockAdvance(2));
            }
            5 => {
                out.push(Action::DcsCas {
                    key_tag: (rng.gen_range(8) as u8).saturating_add(1),
                    val_tag: rng.gen_range(256) as u8,
                    expected_rev: 1 + rng.gen_range(3),
                });
                out.push(Action::ClockAdvance(2));
            }
            6 => {
                let node = 1 + rng.gen_range(n_nodes.max(1));
                out.push(Action::Partition { node });
                out.push(Action::ClockAdvance(5));
            }
            7 => {
                let node = 1 + rng.gen_range(n_nodes.max(1));
                out.push(Action::Heal { node });
                out.push(Action::ClockAdvance(8));
            }
            8 => {
                // Disk fail mid-put.
                let node = 1 + rng.gen_range(n_nodes.max(1));
                let after_ops = rng.gen_range(4);
                let transient = rng.gen_range(2) == 0;
                out.push(Action::DiskArm {
                    node,
                    after_ops,
                    transient,
                });
                out.push(Action::Put {
                    key_tag: (rng.gen_range(16) as u8).saturating_add(1),
                    val_tag: rng.gen_range(256) as u8,
                });
                out.push(Action::ClockAdvance(4));
                out.push(Action::DiskDisarm { node });
                out.push(Action::ClockAdvance(4));
            }
            9 => {
                // Remove + later re-add (catch-up).
                let node = 1 + rng.gen_range(n_nodes.max(1));
                out.push(Action::RemoveMember { node });
                out.push(Action::ClockAdvance(3));
                out.push(Action::Put {
                    key_tag: (rng.gen_range(16) as u8).saturating_add(1),
                    val_tag: rng.gen_range(256) as u8,
                });
                out.push(Action::ClockAdvance(5));
                out.push(Action::AddMember { node });
                out.push(Action::ClockAdvance(10));
            }
            10 => {
                out.push(Action::NetSpray {
                    count: 1 + rng.gen_range(4) as u8,
                });
                out.push(Action::NetTick(1 + rng.gen_range(3) as u32));
                out.push(Action::NetDrain);
            }
            11 => {
                out.push(Action::CommitUnknown {
                    key_tag: (rng.gen_range(16) as u8).saturating_add(1),
                });
                out.push(Action::ClockAdvance(6));
            }
            _ => {
                out.push(Action::ClockAdvance(3));
            }
        }
    }
    // Heal membership + disk; re-add everyone; final ticks.
    for id in 1..=n_nodes {
        out.push(Action::Heal { node: id });
        out.push(Action::DiskDisarm { node: id });
        out.push(Action::AddMember { node: id });
    }
    out.push(Action::ClockAdvance(30));
    out
}

/// RFC-0059 P2.1: splice deterministic membership upgrade/rollback
/// windows into an already-built schedule. Positions are fractions of
/// the ORIGINAL length (pure function of `actions.len()` + `n_nodes`),
/// so the same seed still yields the same action stream.
///
/// Windows:
/// 1. **rolling upgrade under load** — each node exits in order, client
///    writes continue, the node rejoins and catches up (install-snapshot
///    path);
/// 2. **aborted upgrade (rollback)** — everyone leaves again in reverse
///    order, one write lands in the degraded cluster, then the rollout
///    rolls forward (rejoin in order);
/// 3. **quorum shrink** — ⌊(n−1)/2⌋ peers out at once; the surviving
///    majority must keep committing; then everyone returns.
pub fn splice_membership_windows(actions: &mut Vec<Action>, n_nodes: u64) {
    if n_nodes < 2 {
        return;
    }
    let len = actions.len();
    let splice_at = |v: &mut Vec<Action>, at: usize, window: Vec<Action>| {
        let tail = v.split_off(at.min(v.len()));
        v.extend(window);
        v.extend(tail);
    };

    // Window 1: rolling upgrade under load.
    let mut w1 = Vec::new();
    for node in 1..=n_nodes {
        w1.push(Action::RemoveMember { node });
        w1.push(Action::ClockAdvance(3));
        w1.push(Action::Put {
            key_tag: 1,
            val_tag: 0xA7,
        });
        w1.push(Action::ClockAdvance(3));
        w1.push(Action::AddMember { node });
        w1.push(Action::ClockAdvance(6));
    }
    splice_at(actions, len / 3, w1);

    // Window 2: aborted upgrade → rollback → roll forward again.
    let mut w2 = Vec::new();
    for node in (1..=n_nodes).rev() {
        w2.push(Action::RemoveMember { node });
        w2.push(Action::ClockAdvance(2));
    }
    w2.push(Action::Put {
        key_tag: 2,
        val_tag: 0xB9,
    });
    w2.push(Action::ClockAdvance(4));
    for node in 1..=n_nodes {
        w2.push(Action::AddMember { node });
        w2.push(Action::ClockAdvance(4));
    }
    splice_at(actions, (2 * len) / 3, w2);

    // Window 3: quorum shrink — surviving majority must keep committing.
    let out = (n_nodes - 1) / 2;
    let mut w3 = Vec::new();
    if out >= 1 {
        for node in 1..=out {
            w3.push(Action::RemoveMember { node });
        }
        w3.push(Action::ClockAdvance(3));
        w3.push(Action::Put {
            key_tag: 3,
            val_tag: 0xC3,
        });
        w3.push(Action::ClockAdvance(4));
        for node in 1..=out {
            w3.push(Action::AddMember { node });
            w3.push(Action::ClockAdvance(5));
        }
    }
    splice_at(actions, (5 * len) / 6, w3);
}

/// RFC-0060 P1.2: inject a deterministic BitFlip after the first puts so a
/// durable file exists. Off by default — pinning historical seeds.
pub fn splice_bitflip_window(actions: &mut Vec<Action>, n_nodes: u64) {
    if pedradb_core::write_admission_kernel::batch_is_empty(actions.len() as u64) || n_nodes == 0 {
        return;
    }
    let at = (actions.len() / 2).max(1);
    actions.insert(
        at.min(actions.len()),
        Action::BitFlip {
            node: 1,
            n_bits: 1,
            apply: true,
        },
    );
}

/// RFC-0068 P1.2: splice JointRemove then `PlantCommittedJoint` of the
/// last node (needs `n_nodes >= 4` so C-old majority 2/3 cannot elect
/// C-new of 4). Off by default — default `schedule_from_seed` stays
/// fingerprint-stable.
pub fn splice_plant_committed_joint(actions: &mut Vec<Action>, n_nodes: u64) {
    if n_nodes < 4
        || pedradb_core::write_admission_kernel::batch_is_empty(actions.len() as u64)
    {
        return;
    }
    let node = n_nodes;
    let at = 1.min(actions.len());
    let window = [
        Action::JointRemove { node },
        Action::ClockAdvance(20),
        Action::PlantCommittedJoint { node },
    ];
    let tail = actions.split_off(at);
    actions.extend(window);
    actions.extend(tail);
}

/// Fold a string into a running FNV-1a 64-bit hash (stable, no extra deps).
#[must_use]
pub fn hash_str(mut h: u64, s: &str) -> u64 {
    if pedradb_core::write_admission_kernel::batch_is_empty(h) {
        h = 0xcbf2_9ce4_8422_2325;
    }
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// Coarse schedule coverage flags (P3.2-lite edge tags for bandit arms).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScheduleCoverage {
    /// Client puts present.
    pub put: bool,
    /// Get / strong get.
    pub get: bool,
    /// DCS create/cas.
    pub dcs: bool,
    /// DCS with non-zero TTL.
    pub dcs_ttl: bool,
    /// Partition / heal.
    pub partition: bool,
    /// remove/add member.
    pub membership: bool,
    /// DiskArm.
    pub disk: bool,
    /// Net spray/tick/drain.
    pub net: bool,
    /// CommitUnknown G2 canary (RFC-0051 P1.2).
    pub commit_unknown: bool,
    /// AdvanceNowMs.
    pub clock_ms: bool,
    /// BitFlip of a durable page (RFC-0060).
    pub bitflip: bool,
}

impl ScheduleCoverage {
    /// Scan a schedule for coverage bits.
    #[must_use]
    pub fn from_actions(actions: &[Action]) -> Self {
        let mut c = Self::default();
        for a in actions {
            match a {
                Action::Put { .. } => c.put = true,
                Action::Get { .. } | Action::GetStrong { .. } => c.get = true,
                Action::DcsCreate { ttl_ms, .. } => {
                    c.dcs = true;
                    if *ttl_ms > 0 {
                        c.dcs_ttl = true;
                    }
                }
                Action::DcsCas { .. } => c.dcs = true,
                Action::Partition { .. } | Action::Heal { .. } => c.partition = true,
                Action::RemoveMember { .. }
                | Action::AddMember { .. }
                | Action::JointRemove { .. }
                | Action::JointAdd { .. }
                | Action::PlantCommittedJoint { .. } => c.membership = true,
                Action::DiskArm { .. } | Action::DiskDisarm { .. } => c.disk = true,
                Action::NetSpray { .. } | Action::NetTick(_) | Action::NetDrain => c.net = true,
                Action::CommitUnknown { .. } => {
                    c.put = true;
                    c.commit_unknown = true;
                }
                Action::AdvanceNowMs { .. } => c.clock_ms = true,
                Action::BitFlip { .. } => c.bitflip = true,
                Action::StoreTicks(_)
                | Action::ClockAdvance(_)
                | Action::CrashReopen
                | Action::FlushAll
                | Action::AttemptDirectRpc => {}
            }
        }
        c
    }

    /// Stable bit mask for novelty tracking.
    #[must_use]
    pub fn mask(self) -> u32 {
        let mut m = 0u32;
        if self.put {
            m |= 1 << 0;
        }
        if self.get {
            m |= 1 << 1;
        }
        if self.dcs {
            m |= 1 << 2;
        }
        if self.dcs_ttl {
            m |= 1 << 3;
        }
        if self.partition {
            m |= 1 << 4;
        }
        if self.membership {
            m |= 1 << 5;
        }
        if self.disk {
            m |= 1 << 6;
        }
        if self.net {
            m |= 1 << 7;
        }
        if self.clock_ms {
            m |= 1 << 8;
        }
        if self.bitflip {
            m |= 1 << 9;
        }
        m
    }

    /// Primary bandit arm label for this schedule (deterministic priority).
    #[must_use]
    pub fn primary_arm(self) -> &'static str {
        if self.disk {
            "disk"
        } else if self.membership {
            "membership"
        } else if self.dcs_ttl {
            "dcs_ttl"
        } else if self.partition {
            "partition"
        } else if self.dcs {
            "dcs"
        } else if self.net {
            "net"
        } else if self.put {
            "put"
        } else {
            "other"
        }
    }
}

/// All bandit arms used by World soak (order = UCB1 init order).
pub const WORLD_ARMS: &[&str] = &[
    "disk",
    "membership",
    "dcs_ttl",
    "partition",
    "dcs",
    "net",
    "put",
    "other",
];

/// Classify a seed's schedule into a **primary** bandit arm (exclusive).
#[must_use]
pub fn arm_for_seed(seed: u64, n_nodes: u64, steps: usize) -> &'static str {
    let sch = schedule_from_seed(seed, n_nodes, steps);
    ScheduleCoverage::from_actions(&sch).primary_arm()
}

/// Whether a seed's schedule **includes** the named fault/workload class
/// (multi-label; preferred for UCB1 seed pick so rare classes are findable).
#[must_use]
pub fn seed_has_arm(seed: u64, n_nodes: u64, steps: usize, arm: &str) -> bool {
    let sch = schedule_from_seed(seed, n_nodes, steps);
    let c = ScheduleCoverage::from_actions(&sch);
    match arm {
        "disk" => c.disk,
        "membership" => c.membership,
        "dcs_ttl" => c.dcs_ttl,
        "partition" => c.partition,
        "dcs" => c.dcs,
        "net" => c.net,
        "put" => c.put && !c.disk && !c.membership,
        "other" => {
            !c.disk && !c.membership && !c.dcs_ttl && !c.partition && !c.dcs && !c.net && !c.put
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_schedule() {
        let a = schedule_from_seed(99, 3, 12);
        let b = schedule_from_seed(99, 3, 12);
        assert_eq!(a, b);
        let c = schedule_from_seed(100, 3, 12);
        assert_ne!(a, c);
    }

    #[test]
    fn schedule_covers_new_actions() {
        let mut has_clock = false;
        let mut has_get = false;
        let mut has_dcs = false;
        let mut has_rm = false;
        let mut has_disk = false;
        for s in 1..400u64 {
            let sch = schedule_from_seed(s, 3, 32);
            for a in &sch {
                match a {
                    Action::ClockAdvance(_) => has_clock = true,
                    Action::Get { .. } | Action::GetStrong { .. } => has_get = true,
                    Action::DcsCreate { .. } | Action::DcsCas { .. } => has_dcs = true,
                    Action::RemoveMember { .. } => has_rm = true,
                    Action::DiskArm { .. } => has_disk = true,
                    _ => {}
                }
            }
            if has_clock && has_get && has_dcs && has_rm && has_disk {
                break;
            }
        }
        assert!(has_clock && has_get && has_dcs && has_rm && has_disk);
    }

    #[test]
    fn schedule_coverage_mask_stable() {
        let sch = schedule_from_seed(42, 3, 16);
        let a = ScheduleCoverage::from_actions(&sch);
        let b = ScheduleCoverage::from_actions(&sch);
        assert_eq!(a.mask(), b.mask());
        assert_eq!(a.primary_arm(), arm_for_seed(42, 3, 16));
        assert!(WORLD_ARMS.contains(&a.primary_arm()));
    }

    #[test]
    fn default_schedule_omits_plant_committed_joint() {
        for s in 0..64u64 {
            let sch = schedule_from_seed(s, 4, 16);
            assert!(
                !sch.iter()
                    .any(|a| matches!(a, Action::PlantCommittedJoint { .. })),
                "default seed {s} must not emit PlantCommittedJoint"
            );
        }
    }

    #[test]
    fn opt_in_splice_emits_plant_committed_joint() {
        let mut sch = schedule_from_seed(42, 4, 16);
        assert!(!sch
            .iter()
            .any(|a| matches!(a, Action::PlantCommittedJoint { .. })));
        splice_plant_committed_joint(&mut sch, 4);
        assert!(
            sch.iter()
                .any(|a| matches!(a, Action::PlantCommittedJoint { node: 4 })),
            "opt-in splice must emit PlantCommittedJoint of node 4"
        );
        assert!(
            sch.iter()
                .any(|a| matches!(a, Action::JointRemove { node: 4 })),
            "opt-in splice must JointRemove 4 before the plant"
        );
    }
}
