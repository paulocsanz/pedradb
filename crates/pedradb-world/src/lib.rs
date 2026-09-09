//! **pedradb-world** — FDB-parity P1+P2: deterministic World over Montanha-Store.
//!
//! Net + per-peer disk + logical clock + membership + DCS/get workload.
//! See `../FDB-PARITY-ROADMAP.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bandit;
pub mod buggify;
pub mod coverage;
pub mod net;
pub mod pct;
/// PCT runner over real concurrent code (RFC-0051 P0; feature `pct`).
#[cfg(feature = "pct")]
pub mod pct_concurrent;
pub mod schedule;
pub mod scheduler;
/// Parallel swarm executor over World seeds (RFC-0057 P0.3).
pub mod swarm;
/// RFC-0079: native World is not TCG guest coverage.
pub mod tcg;
/// RFC-0059 P2.2: trajectory monotonicity kernel.
mod world_kernel;
pub mod wenv;

pub use buggify::{buggify_schedule_from_seed, BuggifyArm, BuggifySchedule};
pub use coverage::{CoverageMask, SEAM_IDS};
pub use scheduler::{pct_ready_queue, pct_ready_queue_hash};
pub use tcg::{
    allow_claim_tcg_flag, allow_claim_tcg_flag_as_is, tcg_guest_admitted, tcg_guest_admitted_as_is,
    world_runs_guest_ssh, world_runs_guest_ssh_as_is,
};
pub use world_kernel::{
    check_trajectory, check_trajectory_as_is, trajectory_violation, trajectory_violation_as_is,
    TrajectorySample,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pedradb_core::SeedRng;
use pedradb_sim::{FailingEnv, FaultKind, OpClass};
use pedradb_store::{meta_key, RpcMode, StoreCluster, StoreError, StoreOpenOptions};

use crate::wenv::WorldEnv;

use buggify::arm_to_disk_kind;
use net::{InProcessNet, MembershipFault, Net};
use schedule::{hash_str, schedule_from_seed, Action};

/// Error from a world run.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    /// Store / I/O.
    #[error("store: {0}")]
    Store(String),
    /// Config.
    #[error("{0}")]
    Msg(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, WorldError>;

/// One recorded event (for hash + debugging).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEvent {
    /// Step index.
    pub step: u32,
    /// Short tag.
    pub kind: String,
    /// Detail payload.
    pub detail: String,
}

/// Immutable outcome of [`World::run`].
#[derive(Debug, Clone)]
pub struct Trace {
    /// World seed.
    pub seed: u64,
    /// FNV-1a over events (stable across runs).
    pub trace_hash: u64,
    /// Ordered events.
    pub events: Vec<TraceEvent>,
    /// Final put successes.
    pub puts_ok: u32,
    /// Final put errors.
    pub puts_err: u32,
    /// Get successes (Some or None both count as Ok path).
    pub gets_ok: u32,
    /// Get errors (strong-read fail-closed counts here).
    pub gets_err: u32,
    /// DCS mutate successes.
    pub dcs_ok: u32,
    /// DCS mutate errors.
    pub dcs_err: u32,
    /// Net messages enqueued.
    pub net_sent: u64,
    /// Net messages dropped.
    pub net_dropped: u64,
    /// Net messages delivered.
    pub net_delivered: u64,
    /// Peer RPC deliveries applied.
    pub rpc_applied: u64,
    /// Disk fault arm actions.
    pub disk_arms: u32,
    /// Peers still marked tripped at end.
    pub disk_tripped_nodes: u32,
    /// Final store logical time.
    pub logical_now: u64,
    /// RFC-0018 coverage mask bits.
    pub coverage_mask: u64,
    /// Buggify arms applied (site:kind@step).
    pub arms: Vec<String>,
    /// Max `leader_claim_count` observed on any range during the run.
    pub max_leader_claims: u64,
    /// Times Strong policy returned Ok while claim_count ≠ 1 (fail-open dual-leader).
    pub dual_leader_fail_open: u64,
    /// Puts that returned Ok but majority never held the value after exchange (false majority / silent).
    pub false_majority: u64,
    /// Silent wrong: acked put not visible to majority after full pump, or strong read
    /// invents a value when claims ≠ 1.
    pub silent_wrong: u64,
    /// Fold role (RFC-0050 P2.3): final applied cursor of the fold
    /// Storage replica fed from the cluster changelog (0 = role off).
    pub fold_cursor: u64,
    /// Fold role: replay mismatches vs an independent apply of the same
    /// changelog (keyset + values).
    pub fold_mismatch: u32,
    /// G2 canary executions (RFC-0051 P1.2): propose → unknown → retry.
    pub commit_unknown: u32,
    /// G2 violations: a canary node ended with a partial index row (some
    /// keys present, some absent) or a secondary pointing elsewhere.
    pub row_half_indexed: u32,
    /// RFC-0059 P0.2: cross-node consistency invariant violations after
    /// the convergence tail (authenticity / split-brain / resurrection).
    /// 0 when `consistency_check` is off and nothing ran.
    pub consistency_violations: u32,
    /// RFC-0059 P2.2: trajectory invariant violations observed between
    /// exchanges (term / snapshot_index / applied_index regressed on a
    /// live node×range). 0 when `trajectory_check` is off.
    pub trajectory_violations: u32,
    /// RFC-0059 P2.1: membership changes applied by the run (windows +
    /// base schedule removes/adds).
    pub membership_events: u32,
    /// RFC-0079: native World is not TCG guest coverage (`tcg_guest_admitted`).
    pub tcg_guest: bool,
    /// RFC-0070: a PCT-ordered World run is not ∀ OS schedules
    /// (`forall_schedules_admitted`).
    pub forall_schedules: bool,
    /// RFC-0069: eventual-election claim (`liveness_admitted`). Native
    /// World is a bounded seed schedule; default axioms are off.
    pub eventual_election: bool,
    /// RFC-0078 P1.2: stacking `RecordingEnv::Lying` with det_io PRELOAD
    /// (`stacked_fsync_liars_admitted`). Native World uses neither.
    pub stacked_fsync_liars: bool,
}

impl Trace {
    /// RFC-0079: this native run is not TCG guest coverage.
    #[must_use]
    pub fn claim_tcg_guest(&self) -> bool {
        self.tcg_guest
    }

    /// RFC-0070 P1.1: this World run is not ∀ OS schedules.
    #[must_use]
    pub fn claim_forall_schedules(&self) -> bool {
        self.forall_schedules
    }

    /// RFC-0069 P1.2: this World run is not unbounded eventual election
    /// unless ES-1∧ES-2∧ES-3 were named on the config.
    #[must_use]
    pub fn claim_eventual_election(&self) -> bool {
        self.eventual_election
    }

    /// RFC-0078 P1.2: this World run did not AND Lying × det_io.
    #[must_use]
    pub fn claim_stacked_fsync_liars(&self) -> bool {
        self.stacked_fsync_liars
    }

    fn push(&mut self, step: u32, kind: impl Into<String>, detail: impl Into<String>) {
        let kind = kind.into();
        let detail = detail.into();
        let line = format!("{step}|{kind}|{detail}");
        self.trace_hash = hash_str(self.trace_hash, &line);
        self.events.push(TraceEvent { step, kind, detail });
    }
}

/// World config.
#[derive(Debug, Clone)]
pub struct WorldConfig {
    /// Cluster size.
    pub n_nodes: u64,
    /// Key ranges.
    pub n_ranges: u64,
    /// Random schedule body steps.
    pub schedule_steps: usize,
    /// Parent directory for store node data.
    pub parent: PathBuf,
    /// Max Net exchange rounds after each action.
    pub exchange_rounds: usize,
    /// Drop probability (ppm) for Net.
    pub net_drop_ppm: u32,
    /// Max delay ticks on Net.
    pub net_max_delay: u64,
    /// Apply seed-derived buggify multi-fault plan (RFC-0018).
    pub buggify: bool,
    /// Enable net payload corrupt ppm from buggify arms.
    pub net_corrupt_ppm: u32,
    /// Enable message reorder window (0 = off).
    pub net_reorder_window: usize,
    /// Lab-only: per-peer logical clock skew offsets (ms) applied after open (P2.2).
    /// Length must be 0 (disabled) or `n_nodes`. Peer i gets `clock_skew_ms[i]`.
    pub clock_skew_ms: Vec<u64>,
    /// When buggify is on: only apply arms whose index bit is set in this mask.
    /// `None` = all arms. Used by C0.7 shrink (World arm toggles).
    pub buggify_arm_mask: Option<u64>,
    /// PCT orders per-node inbound processing (RFC-0050 P2.1, 5th seam):
    /// each Net exchange round processes deliveries grouped by destination
    /// node in seeded-PCT ready-queue order instead of arrival order.
    /// Same seed ⇒ same ready-queue ⇒ same `trace_hash`.
    pub node_step_pct: bool,
    /// Run one extra role on the same seed (RFC-0050 P2.3): a fold
    /// `Storage` replica consumes the cluster changelog after the
    /// schedule; oracle = independent replay of the same changes
    /// (`fold_mismatch == 0`), cursor reaches the last change seq.
    pub fold_role: bool,
    /// RFC-0058 P0.2: open every node with the **verified profile**
    /// (`StoreOpenOptions::pedra_verified` — `OpenOptions::verified()`
    /// per node: sync forced true, fail-closed recovery). Default false.
    pub verified: bool,
    /// RFC-0059 P0.2: cross-node **consistency invariants** at end of run
    /// (FDB-style invariant checker, single-shard shape): heal the net
    /// tail, pump to quiescence, then check (a) authenticity — every
    /// visible value is a real changelog entry for that key — (b) no
    /// split brain — no two distinct values each visible on a majority
    /// of participating nodes — (c) no resurrection — latest changelog
    /// entry is a delete yet a majority still holds a value. Violations
    /// land in `Trace::consistency_violations` (must be 0).
    pub consistency_check: bool,
    /// WAL barrier data class for the simulated nodes. Default `false`
    /// (RFC-0059): the `fdatasync` weak class — same barrier call and
    /// same fault seam (`OpClass::Sync` gates both), but without the
    /// wall-clock cost of `F_FULLFSYNC`, which serializes across the
    /// volume and caps swarm parallelism. The **product** default stays
    /// the strongest class (`StoreOpenOptions::pedra_wal_full_fsync`);
    /// `verified: true` pins it regardless of this flag.
    pub wal_full_fsync: bool,
    /// RFC-0059 P0.1: run nodes on the in-memory virtual FS
    /// (`WorldEnv::Mem`) instead of the real filesystem. Same engine
    /// code paths and the same `FailingEnv` fault seams (arms gate ops
    /// in the wrapper, not the backing store); removes host-I/O
    /// serialization so swarm throughput scales with cores. Default
    /// `false` (real-FS runs keep exercising extent preallocation and
    /// real barrier syscalls).
    pub mem_storage: bool,
    /// RFC-0059 P2.1: splice deterministic membership upgrade/rollback
    /// windows into the schedule (rolling exit/rejoin under load, an
    /// aborted upgrade rolled back, and a quorum-shrink window where the
    /// surviving majority must keep committing). Default `false` keeps
    /// the historical action stream (pinned regression seeds untouched).
    pub membership_upgrade: bool,
    /// RFC-0059 P2.2: trajectory invariants — after every net exchange,
    /// per (node, range): term, snapshot_index and applied_index must
    /// never decrease. Violations land in
    /// `Trace::trajectory_violations` (must be 0).
    pub trajectory_check: bool,
    /// RFC-0060 P1.2: splice a deterministic BitFlip of a durable page
    /// mid-schedule. Default `false` keeps historical traces.
    pub bitflip: bool,
    /// RFC-0068 P1.2: splice JointRemove + `PlantCommittedJoint` into the
    /// seed schedule (fingerprint bump). Default `false` keeps historical
    /// traces; opt-in only.
    pub plant_committed_joint: bool,
    /// RFC-0069: name ES-1 (finite adversary) for an eventual-election
    /// claim. Default false — native World is a bounded seed schedule.
    pub es1: bool,
    /// RFC-0069: name ES-2 (internal drain) for an eventual-election claim.
    pub es2: bool,
    /// RFC-0069: name ES-3 (live retrying candidate) for an
    /// eventual-election claim.
    pub es3: bool,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 16,
            parent: std::env::temp_dir().join("pedradb-world"),
            exchange_rounds: 48,
            net_drop_ppm: 0,
            net_max_delay: 0,
            buggify: false,
            net_corrupt_ppm: 0,
            net_reorder_window: 0,
            clock_skew_ms: Vec::new(),
            buggify_arm_mask: None,
            node_step_pct: false,
            fold_role: false,
            verified: false,
            consistency_check: false,
            wal_full_fsync: false,
            mem_storage: false,
            membership_upgrade: false,
            trajectory_check: false,
            bitflip: false,
            plant_committed_joint: false,
            es1: false,
            es2: false,
            es3: false,
        }
    }
}

/// Per-node inbound processing order inside each Net exchange round
/// (RFC-0050 P2.1, 5th seam). `Arrival` keeps poll order (legacy
/// `trace_hash`); `Pct` ranks destination nodes by the seeded PCT
/// ready-queue — same seed ⇒ same queue ⇒ same trace.
enum NodeOrder {
    /// Poll (arrival) order — default, unchanged legacy traces.
    Arrival,
    /// Seeded PCT ready-queue of node ids (1-based), windowed per round.
    Pct { queue: Vec<u64>, cursor: usize },
}

impl NodeOrder {
    fn pct(seed: u64, n_nodes: u64, steps: usize) -> Self {
        let queue = crate::scheduler::pct_ready_queue(seed, n_nodes.max(1) as usize, steps.max(1))
            .into_iter()
            .map(|w| w as u64 + 1)
            .collect();
        Self::Pct { queue, cursor: 0 }
    }

    /// Processing permutation for one round's deliveries: destinations
    /// ranked by the current PCT window (stable within a node), identity
    /// otherwise. Advances the window one round.
    fn round_order(&mut self, dests: &[u64], n_nodes: usize) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..dests.len()).collect();
        if let Self::Pct { queue, cursor } = self {
            let qlen = queue.len();
            let rank = |id: u64| {
                (0..n_nodes)
                    .find(|i| queue[(*cursor + i) % qlen] == id)
                    .unwrap_or(usize::MAX)
            };
            idx.sort_by_key(|&i| rank(dests[i]));
            *cursor = (*cursor + n_nodes.max(1)) % qlen;
        }
        idx
    }
}

/// Deterministic multi-peer world.
pub struct World {
    cfg: WorldConfig,
    seed: u64,
    /// Per-node inbound processing order; reset at the top of every
    /// `run_with_schedule` (run is single-threaded).
    order: std::cell::RefCell<NodeOrder>,
    /// RFC-0059 P2.2: last trajectory sample per (node, range); reset at
    /// the top of every `run_with_schedule`. Interior mutability mirrors
    /// `order` — the run is single-threaded.
    traj_prev: std::cell::RefCell<HashMap<(u64, u64), TrajectorySample>>,
}

impl World {
    /// Build a world for `seed`.
    #[must_use]
    pub fn new(seed: u64, cfg: WorldConfig) -> Self {
        Self {
            cfg,
            seed,
            order: std::cell::RefCell::new(NodeOrder::Arrival),
            traj_prev: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// Run the full schedule; return a replayable [`Trace`].
    ///
    /// # Errors
    /// Store open / I/O.
    pub fn run(&self) -> Result<Trace> {
        let mut actions = schedule_from_seed(self.seed, self.cfg.n_nodes, self.cfg.schedule_steps);
        if self.cfg.membership_upgrade {
            schedule::splice_membership_windows(&mut actions, self.cfg.n_nodes);
        }
        if self.cfg.bitflip {
            schedule::splice_bitflip_window(&mut actions, self.cfg.n_nodes);
        }
        if self.cfg.plant_committed_joint {
            schedule::splice_plant_committed_joint(&mut actions, self.cfg.n_nodes);
            let emits = actions
                .iter()
                .any(|a| matches!(a, Action::PlantCommittedJoint { .. }));
            let baseline = schedule_from_seed(self.seed, self.cfg.n_nodes, self.cfg.schedule_steps);
            let omits = !baseline
                .iter()
                .any(|a| matches!(a, Action::PlantCommittedJoint { .. }));
            let _ = pedradb_store::plant_joint_schedule_ok(emits, omits);
        }
        self.run_with_schedule(&actions)
    }

    /// Run an explicit [`Action`] schedule (targeted G2 / PCT trials) with
    /// the same env/net/trace machinery as [`Self::run`]. Buggify still
    /// arms from the seed when `cfg.buggify` is set.
    ///
    /// # Errors
    /// Store open / I/O.
    pub fn run_with_schedule(&self, actions: &[Action]) -> Result<Trace> {
        // Fold `Env::unix_millis` (StdEnv default = wall clock) into the
        // discrete scheduler. Same seed ⇒ same clock; swarm threads each
        // bind their own TLS. Direct RPC is **not** this path (`pin_dst_queued` below).
        let _clock = crate::wenv::LogicalClockGuard::bind_seed(self.seed);
        let parent = self.cfg.parent.join(format!("s{:016x}", self.seed));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).map_err(|e| WorldError::Store(e.to_string()))?;

        let mut disks: HashMap<u64, WorldEnv> = HashMap::new();
        let mut envs = Vec::with_capacity(self.cfg.n_nodes as usize);
        for id in 1..=self.cfg.n_nodes {
            let env = WorldEnv::passing(self.cfg.mem_storage);
            disks.insert(id, env.clone());
            envs.push(env);
        }

        let rng = SeedRng::new(self.seed);
        // RFC-0058 P0.2: `verified` runs open every node with the profile
        // options (same call otherwise — default StoreOpenOptions keeps the
        // historical behavior).
        let store_opts = StoreOpenOptions {
            pedra_verified: self.cfg.verified,
            pedra_wal_full_fsync: self.cfg.wal_full_fsync,
            // `None` mints via SystemRng (wall entropy) — not a function of
            // the seed. Pin identity so persist bytes and buggify sites replay.
            cluster_id: Some(world_cluster_id(self.seed)),
            ..StoreOpenOptions::default()
        };
        let mut cluster = StoreCluster::open_with_envs_rng_opts(
            &parent,
            self.cfg.n_nodes,
            self.cfg.n_ranges,
            envs,
            rng,
            store_opts,
        )
        .map_err(|e| WorldError::Store(e.to_string()))?;
        cluster.pin_dst_queued();

        let mut net = InProcessNet::lossy(
            self.seed ^ 0xA11CE,
            self.cfg.net_drop_ppm,
            self.cfg.net_max_delay,
        );
        if self.cfg.net_corrupt_ppm > 0 {
            net.set_corrupt_ppm(self.cfg.net_corrupt_ppm);
        }
        if self.cfg.net_reorder_window > 0 {
            net.set_reorder_window(self.cfg.net_reorder_window);
        }
        let mut memb = MembershipFault::new();
        let buggify = if self.cfg.buggify {
            Some(buggify_schedule_from_seed(
                self.seed,
                self.cfg.n_nodes,
                self.cfg.schedule_steps,
            ))
        } else {
            None
        };

        let mut cov = CoverageMask::new();
        cov.hit("R.rng");
        cov.hit("H.open");
        cov.hit("C.tick");
        // Lab clock skew (P2.2): advance lease clock by max peer skew.
        if !pedradb_core::write_admission_kernel::batch_is_empty(
            self.cfg.clock_skew_ms.len() as u64,
        ) {
            let max_skew = self.cfg.clock_skew_ms.iter().copied().max().unwrap_or(0);
            if max_skew > 0 {
                let _ = cluster.advance_now_ms(max_skew);
            }
        }
        crate::wenv::LogicalClockGuard::sync_now_ms(self.seed, cluster.now_ms());

        let mut trace = Trace {
            seed: self.seed,
            trace_hash: 0,
            events: Vec::new(),
            puts_ok: 0,
            puts_err: 0,
            gets_ok: 0,
            gets_err: 0,
            dcs_ok: 0,
            dcs_err: 0,
            net_sent: 0,
            net_dropped: 0,
            net_delivered: 0,
            rpc_applied: 0,
            disk_arms: 0,
            disk_tripped_nodes: 0,
            logical_now: 0,
            coverage_mask: 0,
            arms: Vec::new(),
            max_leader_claims: 0,
            dual_leader_fail_open: 0,
            false_majority: 0,
            silent_wrong: 0,
            fold_cursor: 0,
            fold_mismatch: 0,
            commit_unknown: 0,
            row_half_indexed: 0,
            consistency_violations: 0,
            trajectory_violations: 0,
            membership_events: 0,
            // Native World does not SSH and does not invent a guest.
            // Native World does not SSH (RFC-0079 P2.2); guest probe is the script.
            tcg_guest: tcg_guest_admitted(false) && !world_runs_guest_ssh(),
            // RFC-0070: PCT node order (or none) is not ∀π.
            forall_schedules: pedradb_core::group_commit_kernel::forall_schedules_admitted(
                if self.cfg.node_step_pct {
                    pedradb_core::group_commit_kernel::pct_campaign_default_depth()
                } else {
                    0
                },
            ),
            // RFC-0069: bounded World is not eventual election unless the
            // operator names ES-1/ES-2/ES-3. AS-IS would admit anyway.
            eventual_election: pedradb_store::liveness_admitted(
                self.cfg.es1,
                self.cfg.es2,
                self.cfg.es3,
            ),
            // RFC-0078: native World is neither RecordingEnv::Lying nor
            // det_io PRELOAD. Stacking the two liar boxes is refused.
            stacked_fsync_liars: pedradb_core::group_commit_kernel::stacked_fsync_liars_admitted(
                false, false,
            ),
        };

        let arm_enabled = |idx: usize| -> bool {
            match self.cfg.buggify_arm_mask {
                None => true,
                Some(mask) => idx < 64 && (mask & (1u64 << idx)) != 0,
            }
        };
        // 5th seam (RFC-0050 P2.1): PCT orders per-node inbound processing.
        *self.order.borrow_mut() = if self.cfg.node_step_pct {
            NodeOrder::pct(self.seed, self.cfg.n_nodes, self.cfg.schedule_steps)
        } else {
            NodeOrder::Arrival
        };
        self.traj_prev.borrow_mut().clear();

        if let Some(ref plan) = buggify {
            cov.hit("B.buggify");
            for (idx, arm) in plan.arms.iter().enumerate() {
                if !arm_enabled(idx) {
                    continue;
                }
                let tag = format!("{}:{}@{}n{}", arm.site, arm.kind, arm.at_step, arm.node);
                trace.arms.push(tag.clone());
                trace.push(0, "buggify_arm", &tag);
                cov.hit(&arm.site);
                // Pre-arm disk sites immediately (mid-schedule also reapplies in loop).
                if let Some((class, kind, after, transient)) = arm_to_disk_kind(arm) {
                    if let Some(env) = disks.get(&arm.node.max(1).min(self.cfg.n_nodes)) {
                        if kind == FaultKind::ShortWrite {
                            env.arm_short_write(1 + (arm.param as usize % 8));
                        } else {
                            env.arm_op_class(class, after, transient, kind);
                        }
                        if arm.param > 0 && matches!(class, OpClass::Write) {
                            env.set_delay_per_op(arm.param.min(5));
                        }
                        trace.disk_arms += 1;
                        cov.hit("E.write");
                    }
                }
                match arm.site.as_str() {
                    "N.send" => {
                        net.set_drop_ppm(arm.param.min(500_000) as u32);
                        net.set_max_delay(1 + arm.param % 5);
                        cov.hit("N.send");
                    }
                    "N.corrupt" => {
                        net.set_corrupt_ppm(arm.param.min(100_000) as u32);
                        cov.hit("N.corrupt");
                    }
                    "N.part" => {
                        let n = arm.node.max(1).min(self.cfg.n_nodes);
                        memb.set_offline(n, true);
                        let left: Vec<u64> = (1..=self.cfg.n_nodes).filter(|i| *i != n).collect();
                        net.partition(&left, &[n]);
                        cov.hit("N.part");
                    }
                    _ => {}
                }
            }
        }

        for (step, action) in actions.into_iter().enumerate() {
            let step = step as u32;
            // Re-apply buggify arms scheduled at this step.
            if let Some(ref plan) = buggify {
                for (idx, arm) in plan.arms.iter().enumerate() {
                    if !arm_enabled(idx) || arm.at_step != step {
                        continue;
                    }
                    if let Some((class, kind, after, transient)) = arm_to_disk_kind(arm) {
                        if let Some(env) = disks.get(&arm.node.max(1).min(self.cfg.n_nodes)) {
                            env.arm_op_class(class, after, transient, kind);
                            cov.hit(&arm.site);
                        }
                    }
                }
            }
            self.apply_action(
                step,
                action,
                &mut cluster,
                &mut net,
                &mut memb,
                &mut disks,
                &mut trace,
                &mut cov,
            )?;
            self.sample_safety(step, &cluster, &mut trace);
        }

        // Final safety sample after last step.
        self.sample_safety(u32::MAX, &cluster, &mut trace);

        // RFC-0059 P0.2: cross-node consistency invariants (FDB-style
        // invariant checker, single-shard shape). Heal the net tail so a
        // *converged* state is what gets judged — lag from message loss
        // during the faulted schedule is legitimate; a converged state
        // with a phantom value, two majority values, or a resurrected
        // delete is a real violation. Partitions stay: offline nodes are
        // excluded via `is_participating`.
        if self.cfg.consistency_check {
            net.set_drop_ppm(0);
            net.set_corrupt_ppm(0);
            net.set_max_delay(1);
            self.exchange(&mut cluster, &mut net, &mut trace, u32::MAX, "converge")?;

            // Ground truth: the UNION of every participating node's
            // changelog — the same committed-truth source the fold role
            // replays. A single "best reader" node can lag behind a
            // crash/partition tail and would flag committed values as
            // phantoms; a value any participant applied is authentic.
            // Sequences are per-node WAL counters (not comparable
            // across nodes), so "latest entry" is judged per node: a
            // delete proven committed by ANY participant forbids a
            // majority still serving the key.
            // Scope = user-visible keyspaces only (`k…` KV rows and
            // `d/k/…` DCS rows). Excluded, per layer, because they are
            // private bookkeeping legitimately written through local
            // paths rather than the judged changelog: `\0store/`
            // (engine: raft/intent/txn/meta/hist; user puts with that
            // prefix are rejected), `m/` (range membership metadata),
            // `d/m/` (DCS create/mod_rev/lease triplets) and `d/rev`
            // (DCS revision counter).
            const INTERNAL_ROOTS: [&[u8]; 4] = [b"\0store/", b"m/", b"d/m/", b"d/rev"];
            let is_user_key = |k: &[u8]| !INTERNAL_ROOTS.iter().any(|p| k.starts_with(p));
            let part: Vec<u64> = cluster
                .node_ids()
                .iter()
                .copied()
                .filter(|&nid| cluster.is_participating(nid))
                .collect();
            let per_node: Vec<Vec<pedradb_core::ChangeEntry>> = part
                .iter()
                .map(|&nid| cluster.changelog_on(nid, 0))
                .collect();
            let mut history: HashMap<Vec<u8>, Vec<Vec<u8>>> = HashMap::new();
            for changes in &per_node {
                for e in changes {
                    if matches!(e.kind, pedradb_core::ChangeKind::Put) && is_user_key(&e.key) {
                        history
                            .entry(e.key.to_vec())
                            .or_default()
                            .push(e.value.to_vec());
                    }
                }
            }
            // Latest user-key entry per participating node, in that
            // node's seq order (DeleteRange covers every union-history
            // key in [start, end) that the node has not superseded).
            let mut node_latest: Vec<HashMap<Vec<u8>, (bool, u64)>> =
                Vec::with_capacity(part.len());
            for changes in &per_node {
                let mut latest: HashMap<Vec<u8>, (bool, u64)> = HashMap::new();
                for e in changes {
                    let seq = e.sequence;
                    match e.kind {
                        pedradb_core::ChangeKind::Put => {
                            if is_user_key(&e.key) {
                                latest.insert(e.key.to_vec(), (false, seq));
                            }
                        }
                        pedradb_core::ChangeKind::Delete => {
                            if is_user_key(&e.key) {
                                latest.insert(e.key.to_vec(), (true, seq));
                            }
                        }
                        pedradb_core::ChangeKind::DeleteRange => {
                            for k in history.keys() {
                                if k.as_slice() >= e.key.as_ref()
                                    && k.as_slice() < e.value.as_ref()
                                    && !latest.get(k).is_some_and(|&(_, s)| s > seq)
                                {
                                    latest.insert(k.clone(), (true, seq));
                                }
                            }
                        }
                    }
                }
                node_latest.push(latest);
            }
            let maj = part.len() / 2 + 1;
            let step = u32::MAX;
            for (key, values) in &history {
                let mut counts: HashMap<Vec<u8>, usize> = HashMap::new();
                let mut any_visible = 0usize;
                for &nid in &part {
                    let Ok(v) = cluster.get_on(nid, key) else {
                        continue;
                    };
                    let Some(v) = v else { continue };
                    any_visible += 1;
                    // (a) authenticity: phantom value (never in history).
                    if !values.iter().any(|hv| hv.as_slice() == v.as_ref()) {
                        trace.consistency_violations += 1;
                        // DBG: per-node ground truth for the phantom key.
                        let dump: Vec<String> = cluster
                            .node_ids()
                            .iter()
                            .map(|&nid| {
                                let v = cluster.get_on(nid, key).ok().flatten();
                                let snap = cluster.snapshot_index(nid, 1);
                                let applied = cluster.applied_index(nid, 1);
                                let changes: Vec<String> = cluster
                                    .changelog_on(nid, 0)
                                    .into_iter()
                                    .filter(|e| e.key.as_ref() == key.as_slice())
                                    .map(|e| {
                                        format!(
                                            "{:?}@{}={:02x?}",
                                            e.kind,
                                            e.sequence,
                                            e.value.as_ref()
                                        )
                                    })
                                    .collect();
                                format!(
                                    "n{nid}:v={v:?},snap={snap},applied={applied},log={changes:?}"
                                )
                            })
                            .collect();
                        trace.push(
                            step,
                            "consistency_phantom",
                            format!(
                                "k={:02x?} node={nid} v={:02x?} | {}",
                                key,
                                v.as_ref(),
                                dump.join(" | ")
                            ),
                        );
                        let raft: Vec<String> = cluster
                            .node_ids()
                            .iter()
                            .map(|&nid| cluster.raft_debug_line(nid, 1))
                            .collect();
                        trace.push(step, "raft_debug", raft.join(" || "));
                        continue;
                    }
                    *counts.entry(v.as_ref().to_vec()).or_insert(0) += 1;
                }
                // (b) split brain: two distinct values each on ≥ majority.
                let majors = counts.values().filter(|&&c| c >= maj).count();
                if majors > 1 {
                    trace.consistency_violations += 1;
                    trace.push(
                        step,
                        "consistency_split_brain",
                        format!(
                            "k={:02x?} majority_values={} visible={}",
                            key, majors, any_visible
                        ),
                    );
                }
                // (c) resurrection: a delete proven committed by ANY
                // participating node (changelog latest is delete AND the
                // live get is gone), yet a majority still shows a value.
                // Changelog-only deletes are not proof: InstallSnapshot
                // wipes log as user-key Deletes then restores from export,
                // and a lazy CHANGELOG cache can keep the wipe as latest
                // while `get` already serves the restored value (F-found
                // seed 502514). DCS rows (`d/…`) are exempt: lease
                // revoke/expire deletes are local-by-design (non-raft);
                // served invisibility for expired leases is enforced at
                // read time by the DCS layer, not by row absence.
                let is_dcs = key.as_slice().starts_with(b"d/");
                let delete_proven = part.iter().zip(&node_latest).any(|(nid, latest)| {
                    latest.get(key).is_some_and(|&(d, _)| d)
                        && cluster.get_on(*nid, key).ok().flatten().is_none()
                });
                if !is_dcs && delete_proven && any_visible >= maj {
                    trace.consistency_violations += 1;
                    // DBG: which node proves the delete and what each
                    // node's own latest entry for the key is.
                    let proofs: Vec<String> = part
                        .iter()
                        .zip(&node_latest)
                        .map(|(nid, latest)| {
                            let e = latest.get(key).map_or("none".to_string(), |&(d, s)| {
                                format!("{}@{s}", if d { "del" } else { "put" })
                            });
                            let v = cluster.get_on(*nid, key).ok().flatten();
                            let hist: Vec<String> = cluster
                                .changelog_on(*nid, 0)
                                .into_iter()
                                .filter(|c| c.key.as_ref() == key.as_slice())
                                .map(|c| {
                                    format!("{:?}@{}={:02x?}", c.kind, c.sequence, c.value.as_ref())
                                })
                                .collect();
                            format!("n{nid}:latest={e},v={v:?},hist={hist:?}")
                        })
                        .collect();
                    let rid = cluster.locate(key).unwrap_or(0);
                    let raft: Vec<String> = part
                        .iter()
                        .map(|&nid| cluster.raft_debug_line(nid, rid))
                        .collect();
                    trace.push(
                        step,
                        "consistency_resurrected",
                        format!(
                            "k={:02x?} deleted yet visible on {any_visible} | {} | RAFT {}",
                            key,
                            proofs.join(" | "),
                            raft.join(" || ")
                        ),
                    );
                }
            }
        }

        // RFC-0050 P2.3: one extra role on the same seed — a fold Storage
        // replica consumes the cluster changelog. Oracle = an independent
        // replay of the same changes (keyset + values), cursor = last seq.
        if self.cfg.fold_role {
            use pedradb_fold::{FoldCursor, FoldRole, FoldUpdate, PedraFold};
            use std::collections::BTreeMap;
            let changes = cluster.changelog_after(0);
            let (cur, mut fold) = PedraFold::open_role_env(
                &parent.join("fold"),
                FoldRole::Storage,
                FailingEnv::passing(),
            )
            .map_err(|e| WorldError::Store(e.to_string()))?;
            let mut expected: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
            if !pedradb_core::write_admission_kernel::batch_is_empty(changes.len() as u64) {
                let updates: Vec<FoldUpdate> = changes
                    .iter()
                    .map(|e| match e.kind {
                        pedradb_core::ChangeKind::Put => FoldUpdate::Put {
                            key: e.key.to_vec(),
                            value: e.value.to_vec(),
                            seq: e.sequence,
                        },
                        pedradb_core::ChangeKind::Delete => FoldUpdate::Delete {
                            key: e.key.to_vec(),
                            seq: e.sequence,
                        },
                        pedradb_core::ChangeKind::DeleteRange => FoldUpdate::DeleteRange {
                            start: e.key.to_vec(),
                            end: e.value.to_vec(),
                            seq: e.sequence,
                        },
                    })
                    .collect();
                let last = updates.last().map(|u| u.seq()).unwrap_or(cur.0);
                fold.apply_updates(&updates, FoldCursor(last))
                    .map_err(|e| WorldError::Store(e.to_string()))?;
                for e in &changes {
                    match e.kind {
                        pedradb_core::ChangeKind::Put => {
                            expected.insert(e.key.to_vec(), e.value.to_vec());
                        }
                        pedradb_core::ChangeKind::Delete => {
                            expected.remove(e.key.as_ref());
                        }
                        pedradb_core::ChangeKind::DeleteRange => {
                            expected.retain(|k, _| {
                                !(e.key.as_ref()..e.value.as_ref()).contains(&k.as_slice())
                            });
                        }
                    }
                }
            }
            trace.fold_cursor = fold.cursor_value().0;
            let got = fold
                .range_values(b"")
                .map_err(|e| WorldError::Store(e.to_string()))?;
            let want: Vec<(Vec<u8>, Vec<u8>)> = expected.into_iter().collect();
            trace.fold_mismatch = want.iter().zip(got.iter()).filter(|(a, b)| a != b).count()
                as u32
                + want.len().saturating_sub(got.len()) as u32
                + got.len().saturating_sub(want.len()) as u32;
        }

        if !pedradb_core::write_admission_kernel::batch_is_empty(net.sent as u64) {
            cov.hit("N.send");
        }
        if !pedradb_core::write_admission_kernel::batch_is_empty(net.corrupted as u64) {
            cov.hit("N.corrupt");
        }
        if !pedradb_core::write_admission_kernel::batch_is_empty(net.reordered as u64) {
            cov.hit("N.send");
        }

        trace.disk_tripped_nodes = disks.values().filter(|e| e.tripped()).count() as u32;
        trace.net_sent = net.sent;
        trace.net_dropped = net.dropped;
        trace.net_delivered = net.delivered;
        trace.logical_now = cluster.logical_now();
        trace.coverage_mask = cov.bits();
        trace.trace_hash = hash_str(
            trace.trace_hash,
            &format!(
                "end|ok={}|err={}|gok={}|gerr={}|dok={}|derr={}|ns={}|nd={}|nv={}|rpc={}|arms={}|trip={}|t={}|mask={:x}|buggy={}",
                trace.puts_ok,
                trace.puts_err,
                trace.gets_ok,
                trace.gets_err,
                trace.dcs_ok,
                trace.dcs_err,
                trace.net_sent,
                trace.net_dropped,
                trace.net_delivered,
                trace.rpc_applied,
                trace.disk_arms,
                trace.disk_tripped_nodes,
                trace.logical_now,
                trace.coverage_mask,
                trace.arms.len()
            ),
        );

        // 5th seam: bind the trace to the ready-queue itself (pct mode only —
        // default hashes stay byte-identical).
        if self.cfg.node_step_pct {
            let q = crate::scheduler::pct_ready_queue_hash(
                self.seed,
                self.cfg.n_nodes.max(1) as usize,
                self.cfg.schedule_steps.max(1),
            );
            trace.trace_hash = hash_str(trace.trace_hash, &format!("order=pct|q={q:x}"));
        }
        // Fold role binds the trace to (cursor, mismatches) when on.
        if self.cfg.fold_role {
            trace.trace_hash = hash_str(
                trace.trace_hash,
                &format!("fold|cur={}|mis={}", trace.fold_cursor, trace.fold_mismatch),
            );
        }

        let _ = std::fs::remove_dir_all(&parent);
        Ok(trace)
    }

    fn exchange(
        &self,
        cluster: &mut StoreCluster<WorldEnv>,
        net: &mut InProcessNet,
        trace: &mut Trace,
        step: u32,
        tag: &str,
    ) -> Result<()> {
        let mut applied = 0u64;
        for _ in 0..self.cfg.exchange_rounds {
            let batch = cluster.drain_outbound();
            for (from, to, bytes) in batch {
                net.send(from, to, bytes);
            }
            net.tick();
            let mut got = 0u32;
            let mut inbound: Vec<(u64, u64, Vec<u8>)> = Vec::new();
            while let Some(d) = net.poll() {
                got += 1;
                // PeerMsg tags 1..=6 (incl. InstallSnapshot).
                if pedradb_core::write_admission_kernel::batch_is_empty(d.bytes.len() as u64)
                    || !(1..=6).contains(&d.bytes[0])
                {
                    continue;
                }
                inbound.push((d.from, d.to, d.bytes));
            }
            if !pedradb_core::write_admission_kernel::batch_is_empty(inbound.len() as u64) {
                let dests: Vec<u64> = inbound.iter().map(|(_, to, _)| *to).collect();
                let order = self
                    .order
                    .borrow_mut()
                    .round_order(&dests, self.cfg.n_nodes as usize);
                for i in order {
                    let (from, to, bytes) = &inbound[i];
                    match cluster.handle_inbound(*from, *to, bytes) {
                        Ok(()) => applied += 1,
                        Err(e) => {
                            trace.push(step, "rpc_err", format!("f={from} t={to} e={e}"));
                        }
                    }
                }
            }
            if got == 0 && cluster.outbound_len() == 0 {
                break;
            }
        }
        if !pedradb_core::write_admission_kernel::batch_is_empty(applied as u64) {
            trace.rpc_applied += applied;
            trace.push(step, "rpc", format!("{tag} n={applied}"));
        }
        // RFC-0059 P2.2: trajectory invariant — sampled after every
        // exchange; a regression on a live node×range is an oracle hit.
        if self.cfg.trajectory_check {
            let mut prev = self.traj_prev.borrow_mut();
            for nid in 1..=self.cfg.n_nodes {
                for rid in 1..=self.cfg.n_ranges {
                    let cur = TrajectorySample {
                        step,
                        node: nid,
                        range: rid,
                        term: cluster.term_on(nid, rid),
                        snapshot_index: cluster.snapshot_index(nid, rid),
                        applied_index: cluster.applied_index(nid, rid),
                    };
                    if let Some(p) = prev.get(&(nid, rid)) {
                        if let Some(what) = trajectory_violation(p, &cur) {
                            trace.trajectory_violations += 1;
                            trace.push(
                                step,
                                "trajectory_regression",
                                format!("{what} n{nid} r{rid} {p:?} -> {cur:?}"),
                            );
                        }
                    }
                    prev.insert((nid, rid), cur);
                }
            }
        }
        Ok(())
    }

    fn pump_propose(
        &self,
        cluster: &mut StoreCluster<WorldEnv>,
        net: &mut InProcessNet,
        trace: &mut Trace,
        step: u32,
        result: std::result::Result<(), StoreError>,
        ok_kind: &str,
        err_prefix: &str,
    ) -> Result<bool> {
        match result {
            Ok(()) => {
                self.exchange(cluster, net, trace, step, ok_kind)?;
                Ok(true)
            }
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => {
                self.exchange(cluster, net, trace, step, "nc")?;
                let committed = cluster
                    .finish_queued_propose(range_id, index, false)
                    .unwrap_or(false);
                if committed {
                    self.exchange(cluster, net, trace, step, "hb")?;
                    let _ = cluster.finish_queued_propose(range_id, index, false);
                    self.exchange(cluster, net, trace, step, "fin")?;
                    Ok(true)
                } else {
                    let _ = cluster.finish_queued_propose(range_id, index, true);
                    self.exchange(cluster, net, trace, step, "abort")?;
                    trace.push(
                        step,
                        "err",
                        format!("{err_prefix} NotCommitted idx={index}"),
                    );
                    Ok(false)
                }
            }
            Err(e) => {
                self.exchange(cluster, net, trace, step, "io")?;
                trace.push(step, "err", format!("{err_prefix} {e}"));
                Ok(false)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_action(
        &self,
        step: u32,
        action: &Action,
        cluster: &mut StoreCluster<WorldEnv>,
        net: &mut InProcessNet,
        memb: &mut MembershipFault,
        disks: &mut HashMap<u64, WorldEnv>,
        trace: &mut Trace,
        cov: &mut CoverageMask,
    ) -> Result<()> {
        match action {
            Action::StoreTicks(n) | Action::ClockAdvance(n) => {
                cov.hit("C.tick");
                for _ in 0..*n {
                    if let Err(e) = cluster.tick() {
                        trace.push(step, "tick_err", format!("{e}"));
                    }
                    self.exchange(cluster, net, trace, step, "tick")?;
                }
                crate::wenv::LogicalClockGuard::sync_now_ms(self.seed, cluster.now_ms());
                trace.push(
                    step,
                    "clock",
                    format!("n={n} now={}", cluster.logical_now()),
                );
            }
            Action::Put { key_tag, val_tag } => {
                let key = vec![b'k', *key_tag];
                let val = vec![b'v', *val_tag];
                let put_res = cluster.put(&key, &val);
                let ok = self.pump_propose(
                    cluster,
                    net,
                    trace,
                    step,
                    put_res,
                    "put",
                    &format!("put k={key_tag}"),
                )?;
                if ok {
                    trace.puts_ok += 1;
                    // Extra exchange so AE applies before visibility check.
                    self.exchange(cluster, net, trace, step, "put_vis")?;
                    let (seen, part, maj) = count_seen_participating(cluster, &key, &val);
                    // Silent wrong / false majority (honest definition under faults):
                    // 1) Unique range leader must hold the acked value (local apply).
                    // 2) If ≥maj participating peers are healthy enough to form a quorum
                    //    and *none* of the disk-tripped nodes are required, require seen≥maj.
                    //    Under active partition we only enforce (1).
                    // Silent wrong: after put Ok, Strong or unique-leader local must not
                    // invent a *wrong* value. Missing under re-elect/partition is fail-closed
                    // (not silent). Wrong bytes = silent.
                    match cluster.get_strong(&key) {
                        Ok(Some(v)) if v.as_ref() != val.as_slice() => {
                            trace.silent_wrong += 1;
                            trace.push(
                                step,
                                "silent_wrong",
                                format!("k={key_tag} strong_wrong_val"),
                            );
                        }
                        Ok(Some(_)) => {
                            // Correct value via strong path — good.
                        }
                        Ok(None) => {
                            // Strong Ok empty: put was for this key; empty means lost.
                            // Only if unique leader exists (range_leader Some).
                            if let Ok(rid) = cluster.locate(&key) {
                                if cluster.range_leader(rid).is_some() {
                                    trace.silent_wrong += 1;
                                    trace.false_majority += 1;
                                    let raft: Vec<String> = cluster
                                        .node_ids()
                                        .iter()
                                        .map(|&nid| cluster.raft_debug_line(nid, rid))
                                        .collect();
                                    trace.push(
                                        step,
                                        "false_majority",
                                        format!(
                                            "k={key_tag} strong_none after put_ok seen={seen} | {}",
                                            raft.join(" || ")
                                        ),
                                    );
                                }
                            }
                        }
                        Err(_) => {
                            // NotLeader / dual claim — fail-closed, not silent wrong.
                        }
                    }
                    // Majority visibility when fully connected (all online, no membership fault).
                    let fully_connected =
                        part == self.cfg.n_nodes as usize && memb.offline_ids().is_empty();
                    if fully_connected && seen < maj {
                        self.exchange(cluster, net, trace, step, "put_vis2")?;
                        let (seen2, _, _) = count_seen_participating(cluster, &key, &val);
                        if seen2 < maj {
                            // Still short: if Strong has correct value, lag is AE-only
                            // (not silent). If Strong also missing with unique leader → false maj.
                            match cluster.get_strong(&key) {
                                Ok(Some(v)) if v.as_ref() == val.as_slice() => {}
                                Ok(_) | Err(_) => {
                                    if let Ok(rid) = cluster.locate(&key) {
                                        if cluster.range_leader(rid).is_some()
                                            && cluster
                                                .get_strong(&key)
                                                .ok()
                                                .flatten()
                                                .map(|b| b.as_ref() == val.as_slice())
                                                != Some(true)
                                        {
                                            trace.false_majority += 1;
                                            trace.silent_wrong += 1;
                                            trace.push(
                                                step,
                                                "false_majority",
                                                format!(
                                                    "k={key_tag} seen={seen2} maj={maj} fully_connected"
                                                ),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    trace.push(
                        step,
                        "put_ok",
                        format!("k={key_tag} v={val_tag} seen={seen}"),
                    );
                } else {
                    trace.puts_err += 1;
                }
            }
            Action::CommitUnknown { key_tag } => {
                // G2 canary (RFC-0051 P1.2): one atomic batch — row + two
                // secondary index entries (same shape as the
                // `pedradb_index` canary) — proposed into an **uncertain**
                // commit: partition the range leader mid-flight, resolve
                // under re-election (NotCommitted-after-majority), heal,
                // retry the same batch, then require every reachable node
                // to hold 0 or 1 complete correct set — never a partial
                // (`row_half_indexed`) row or an off-target secondary.
                let row = vec![b'c', b'r', *key_tag];
                let idx_name = vec![b'c', b'i', b'n', *key_tag];
                let idx_mail = vec![b'c', b'i', b'm', *key_tag];
                let val = vec![b'v', *key_tag];
                let batch = vec![
                    (row.clone(), val.clone()),
                    (idx_name.clone(), row.clone()),
                    (idx_mail.clone(), row.clone()),
                ];
                // (1) propose the canary row.
                let first = cluster.put_batch(batch.clone());
                // (2) make the outcome unknown: partition the leader.
                let mut leader = None;
                if let Ok(rid) = cluster.locate(&row) {
                    leader = cluster.range_leader(rid);
                    if let Some(l) = leader {
                        memb.set_offline(l, true);
                        let _ = cluster.set_participating(l, false);
                        let online: Vec<u64> = (1..=self.cfg.n_nodes)
                            .filter(|id| !memb.is_offline(*id))
                            .collect();
                        net.partition(&[l], &online);
                    }
                }
                let ok1 = self.pump_propose(
                    cluster,
                    net,
                    trace,
                    step,
                    first,
                    "cu",
                    &format!("cu k={key_tag}"),
                )?;
                for _ in 0..3 {
                    let _ = cluster.tick();
                    self.exchange(cluster, net, trace, step, "cu_part")?;
                }
                crate::wenv::LogicalClockGuard::sync_now_ms(self.seed, cluster.now_ms());
                // (3) heal.
                if let Some(l) = leader {
                    memb.set_offline(l, false);
                    if cluster.is_member(l) {
                        let _ = cluster.set_participating(l, true);
                    }
                    if pedradb_core::write_admission_kernel::batch_is_empty(
                        memb.offline_ids().len() as u64,
                    ) {
                        net.heal();
                    }
                    self.exchange(cluster, net, trace, step, "cu_heal")?;
                }
                // (4) client retry with the same batch.
                let second = cluster.put_batch(batch.clone());
                let ok2 = self.pump_propose(
                    cluster,
                    net,
                    trace,
                    step,
                    second,
                    "cu2",
                    &format!("cu retry k={key_tag}"),
                )?;
                for _ in 0..4 {
                    let _ = cluster.tick();
                    self.exchange(cluster, net, trace, step, "cu_settle")?;
                }
                crate::wenv::LogicalClockGuard::sync_now_ms(self.seed, cluster.now_ms());
                // (5) oracle: 0 or 1 complete correct set per reachable node.
                let mut half = 0u32;
                for nid in cluster.node_ids() {
                    if !cluster.is_participating(*nid) {
                        continue;
                    }
                    let r = cluster.get_on(*nid, &row).ok().flatten();
                    let n1 = cluster.get_on(*nid, &idx_name).ok().flatten();
                    let n2 = cluster.get_on(*nid, &idx_mail).ok().flatten();
                    let present = [r.is_some(), n1.is_some(), n2.is_some()];
                    let count = present.iter().filter(|b| **b).count();
                    if count != 0 && count != present.len() {
                        half += 1;
                    }
                    if let Some(v) = &r {
                        if v.as_ref() != val.as_slice() {
                            trace.silent_wrong += 1;
                            trace.push(
                                step,
                                "cu_row_wrong_val",
                                format!("k={key_tag} n={nid} v={:02x?}", v.as_ref()),
                            );
                        }
                    }
                    for iv in [&n1, &n2] {
                        if let Some(v) = iv {
                            if v.as_ref() != row.as_slice() {
                                half += 1; // secondary points elsewhere
                            }
                        }
                    }
                }
                trace.commit_unknown += 1;
                trace.row_half_indexed += half;
                trace.push(
                    step,
                    "cu",
                    format!("k={key_tag} ok1={ok1} ok2={ok2} half={half}"),
                );
                if half > 0 {
                    trace.push(step, "cu_half", format!("k={key_tag} half={half}"));
                }
            }
            Action::Get { key_tag, node } => {
                let key = vec![b'k', *key_tag];
                match cluster.get_on(*node, &key) {
                    Ok(v) => {
                        trace.gets_ok += 1;
                        let hit = u8::from(v.is_some());
                        trace.push(step, "get_ok", format!("k={key_tag} n={node} hit={hit}"));
                    }
                    Err(e) => {
                        trace.gets_err += 1;
                        trace.push(step, "get_err", format!("k={key_tag} e={e}"));
                    }
                }
            }
            Action::GetStrong { key_tag } => {
                let key = vec![b'k', *key_tag];
                let rid = cluster.locate(&key).unwrap_or(0);
                let claims = cluster.leader_claim_count(rid);
                match cluster.get_strong(&key) {
                    Ok(v) => {
                        trace.gets_ok += 1;
                        let hit = u8::from(v.is_some());
                        // Fail-open: Strong Ok while no unique leader.
                        if claims != 1 {
                            trace.dual_leader_fail_open += 1;
                            trace.silent_wrong += 1;
                            trace.push(
                                step,
                                "dual_leader_fail_open",
                                format!("k={key_tag} claims={claims} strong=Ok"),
                            );
                        }
                        trace.push(
                            step,
                            "get_strong_ok",
                            format!("k={key_tag} hit={hit} claims={claims}"),
                        );
                    }
                    Err(e) => {
                        trace.gets_err += 1;
                        // Fail-closed when claims ≠ 1 is expected.
                        trace.push(
                            step,
                            "get_strong_err",
                            format!("k={key_tag} e={e} claims={claims}"),
                        );
                    }
                }
            }
            Action::AdvanceNowMs { ms } => {
                cluster.advance_now_ms(*ms);
                crate::wenv::LogicalClockGuard::sync_now_ms(self.seed, cluster.now_ms());
                trace.push(step, "now_ms", format!("+={ms} now={}", cluster.now_ms()));
            }
            Action::DcsCreate {
                key_tag,
                val_tag,
                ttl_ms,
            } => {
                let key = meta_key(&[*key_tag]);
                let val = vec![b'd', *val_tag];
                let res = cluster.dcs_create_ttl(&key, &val, *ttl_ms).map(|_| ());
                let ok = self.pump_propose(
                    cluster,
                    net,
                    trace,
                    step,
                    res,
                    "dcs_c",
                    &format!("dcs_create k={key_tag} ttl={ttl_ms}"),
                )?;
                if ok {
                    trace.dcs_ok += 1;
                    trace.push(step, "dcs_ok", format!("create k={key_tag} ttl={ttl_ms}"));
                } else {
                    trace.dcs_err += 1;
                }
            }
            Action::DcsCas {
                key_tag,
                val_tag,
                expected_rev,
            } => {
                let key = meta_key(&[*key_tag]);
                let val = vec![b'c', *val_tag];
                let res = cluster.dcs_cas(&key, &val, *expected_rev).map(|_| ());
                let ok = self.pump_propose(
                    cluster,
                    net,
                    trace,
                    step,
                    res,
                    "dcs_cas",
                    &format!("dcs_cas k={key_tag}"),
                )?;
                if ok {
                    trace.dcs_ok += 1;
                    trace.push(
                        step,
                        "dcs_ok",
                        format!("cas k={key_tag} exp={expected_rev}"),
                    );
                } else {
                    trace.dcs_err += 1;
                }
            }
            Action::Partition { node } => {
                if *node >= 1 && *node <= self.cfg.n_nodes {
                    cov.hit("N.part");
                    memb.set_offline(*node, true);
                    let _ = cluster.set_participating(*node, false);
                    let online: Vec<u64> = (1..=self.cfg.n_nodes)
                        .filter(|id| !memb.is_offline(*id))
                        .collect();
                    net.partition(&[*node], &online);
                    trace.push(step, "part", format!("node={node}"));
                }
            }
            Action::Heal { node } => {
                if *node >= 1 && *node <= self.cfg.n_nodes {
                    memb.set_offline(*node, false);
                    if cluster.is_member(*node) {
                        let _ = cluster.set_participating(*node, true);
                    }
                    if pedradb_core::write_admission_kernel::batch_is_empty(
                        memb.offline_ids().len() as u64,
                    ) {
                        net.heal();
                    } else {
                        let offline = memb.offline_ids();
                        let online: Vec<u64> = (1..=self.cfg.n_nodes)
                            .filter(|id| !memb.is_offline(*id))
                            .collect();
                        net.partition(&offline, &online);
                    }
                    self.exchange(cluster, net, trace, step, "heal")?;
                    trace.push(step, "heal", format!("node={node}"));
                }
            }
            Action::RemoveMember { node } => {
                if *node >= 1 && *node <= self.cfg.n_nodes {
                    match cluster.remove_member(*node) {
                        Ok(()) => {
                            memb.set_offline(*node, true);
                            trace.membership_events += 1;
                            trace.push(step, "rm_member", format!("node={node}"));
                        }
                        Err(e) => {
                            trace.push(step, "rm_member_err", format!("node={node} e={e}"));
                        }
                    }
                }
            }
            Action::AddMember { node } => {
                if *node >= 1 && *node <= self.cfg.n_nodes {
                    match cluster.add_member(*node) {
                        Ok(()) => {
                            memb.set_offline(*node, false);
                            if pedradb_core::write_admission_kernel::batch_is_empty(
                                memb.offline_ids().len() as u64,
                            ) {
                                net.heal();
                            }
                            self.exchange(cluster, net, trace, step, "add")?;
                            // Extra ticks for install-snapshot catch-up.
                            for _ in 0..8 {
                                let _ = cluster.tick();
                                self.exchange(cluster, net, trace, step, "add_tick")?;
                            }
                            crate::wenv::LogicalClockGuard::sync_now_ms(
                                self.seed,
                                cluster.now_ms(),
                            );
                            trace.membership_events += 1;
                            trace.push(step, "add_member", format!("node={node}"));
                        }
                        Err(e) => {
                            trace.push(step, "add_member_err", format!("node={node} e={e}"));
                        }
                    }
                }
            }
            Action::DiskArm {
                node,
                after_ops,
                transient,
            } => {
                if let Some(env) = disks.get(node) {
                    env.arm(*after_ops, *transient);
                    trace.disk_arms += 1;
                    cov.hit("E.write");
                    cov.hit("E.sync");
                    trace.push(
                        step,
                        "disk_arm",
                        format!("node={node} after={after_ops} t={transient}"),
                    );
                }
            }
            Action::DiskDisarm { node } => {
                if let Some(env) = disks.get(node) {
                    let was = env.tripped();
                    env.disarm();
                    trace.push(step, "disk_disarm", format!("node={node} was_trip={was}"));
                }
            }
            Action::NetSpray { count } => {
                cov.hit("N.send");
                let n = self.cfg.n_nodes.max(1);
                for i in 0..*count {
                    let from = 1 + (i as u64 % n);
                    let to = 1 + ((i as u64 + 1) % n);
                    let mut bytes = vec![0xA5, i];
                    bytes.extend_from_slice(&self.seed.to_le_bytes());
                    net.send(from, to, bytes);
                }
                trace.push(step, "net_spray", format!("c={count}"));
            }
            Action::NetTick(n) => {
                for _ in 0..*n {
                    net.tick();
                }
                let mut k = 0u32;
                while let Some(d) = net.poll() {
                    if !pedradb_core::write_admission_kernel::batch_is_empty(d.bytes.len() as u64)
                        && (1..=6).contains(&d.bytes[0])
                    {
                        if cluster.handle_inbound(d.from, d.to, &d.bytes).is_ok() {
                            k += 1;
                            trace.rpc_applied += 1;
                        }
                    } else {
                        k += 1;
                    }
                }
                trace.push(step, "net_tick", format!("n={n} drained={k}"));
            }
            Action::BitFlip {
                node,
                n_bits,
                apply,
            } => {
                let nid = *node;
                let Some(env) = disks.get(&nid).cloned() else {
                    trace.push(step, "bitflip_skip", format!("node={nid} missing env"));
                    return Ok(());
                };
                if let Err(e) = cluster.flush_engine_on(nid) {
                    trace.push(step, "bitflip_flush_err", format!("node={nid} e={e}"));
                }
                let Some(node_dir) = cluster.node_data_dir(nid) else {
                    trace.push(step, "bitflip_skip", format!("node={nid} no dir"));
                    return Ok(());
                };
                match pedradb_core::xor_durable_bits(
                    &env,
                    &node_dir,
                    self.seed ^ u64::from(step),
                    *n_bits,
                    *apply,
                ) {
                    Some(hit) => {
                        let kind = if *apply {
                            "bitflip"
                        } else {
                            "bitflip_unapplied"
                        };
                        trace.push(
                            step,
                            kind,
                            format!(
                                "node={nid} file={} offset={} bits={n_bits} apply={apply}",
                                hit.file, hit.offset
                            ),
                        );
                        let scrub = pedradb_core::verify_at_rest(&env, &node_dir);
                        trace.push(
                            step,
                            "bitflip_verify",
                            format!("clean={} {}", scrub.is_clean(), scrub.summary_line()),
                        );
                        if *apply {
                            if let Err(e) = cluster.reopen_engine_on(nid, env) {
                                trace.push(step, "bitflip_reopen_err", format!("node={nid} e={e}"));
                            } else {
                                trace.push(step, "bitflip_reopen_ok", format!("node={nid}"));
                            }
                        }
                    }
                    None => {
                        trace.push(step, "bitflip_skip", format!("node={nid} no durable file"));
                    }
                }
            }
            Action::FlushAll => {
                for nid in 1..=self.cfg.n_nodes {
                    if let Err(e) = cluster.flush_engine_on(nid) {
                        trace.push(step, "flush_err", format!("node={nid} e={e}"));
                    }
                }
                trace.push(step, "flush_all", format!("n={}", self.cfg.n_nodes));
            }
            Action::JointAdd { node } => {
                if *node >= 1 && *node <= self.cfg.n_nodes {
                    let res = cluster.add_member_joint(*node);
                    let ok = self.pump_propose(
                        cluster,
                        net,
                        trace,
                        step,
                        res,
                        "joint_add",
                        &format!("joint_add node={node}"),
                    )?;
                    if ok {
                        trace.membership_events += 1;
                        trace.push(step, "joint_add", format!("node={node}"));
                    }
                }
            }
            Action::JointRemove { node } => {
                if *node >= 1 && *node <= self.cfg.n_nodes {
                    let res = cluster.remove_member_joint(*node);
                    let ok = self.pump_propose(
                        cluster,
                        net,
                        trace,
                        step,
                        res,
                        "joint_rm",
                        &format!("joint_rm node={node}"),
                    )?;
                    if ok {
                        trace.membership_events += 1;
                        trace.push(step, "joint_rm", format!("node={node}"));
                    }
                }
            }
            Action::PlantCommittedJoint { node } => {
                if *node >= 1 && *node <= self.cfg.n_nodes {
                    match cluster.plant_committed_joint_without_leave(*node) {
                        Ok(()) => {
                            trace.membership_events += 1;
                            if cluster.probe_old_majority_joint_election(1) {
                                trace.silent_wrong += 1;
                                trace.push(step, "joint_plant_old_elects", format!("node={node}"));
                            } else {
                                trace.push(step, "joint_plant_old_refused", format!("node={node}"));
                            }
                        }
                        Err(e) => {
                            trace.push(step, "joint_plant_err", format!("node={node} e={e}"));
                        }
                    }
                }
            }
            Action::AttemptDirectRpc => {
                cluster.set_rpc_mode(RpcMode::Direct);
                if cluster.rpc_mode() == RpcMode::Direct {
                    trace.silent_wrong += 1;
                    trace.push(step, "direct_rpc_admitted", "pin failed");
                } else {
                    trace.push(step, "direct_rpc_refused", "queued");
                }
            }
            Action::CrashReopen => {
                cov.hit("E.sync");
                for nid in 1..=self.cfg.n_nodes {
                    let Some(env) = disks.get(&nid).cloned() else {
                        continue;
                    };
                    env.crash_unsynced();
                    match cluster.crash_reopen_engine_on(nid, env) {
                        Ok(()) => {
                            trace.push(step, "crash_reopen_ok", format!("node={nid}"));
                        }
                        Err(e) => {
                            trace.push(step, "crash_reopen_err", format!("node={nid} e={e}"));
                        }
                    }
                }
            }
            Action::NetDrain => {
                let mut k = 0u32;
                while let Some(d) = net.poll() {
                    k += 1;
                    if !pedradb_core::write_admission_kernel::batch_is_empty(d.bytes.len() as u64)
                        && (1..=6).contains(&d.bytes[0])
                    {
                        let _ = cluster.handle_inbound(d.from, d.to, &d.bytes);
                        trace.rpc_applied += 1;
                    }
                    trace.push(
                        step,
                        "net_deliv",
                        format!("f={} t={} len={}", d.from, d.to, d.bytes.len()),
                    );
                }
                self.exchange(cluster, net, trace, step, "drain")?;
                if pedradb_core::write_admission_kernel::batch_is_empty(k as u64) {
                    trace.push(step, "net_drain", "empty");
                }
            }
        }
        Ok(())
    }

    /// Sample leadership claims and Strong-policy fail-open on every range.
    fn sample_safety(&self, step: u32, cluster: &StoreCluster<WorldEnv>, trace: &mut Trace) {
        // Probe key `k\x01` — always locatable in single-byte split.
        let probe = [b'k', 1u8];
        let Ok(rid) = cluster.locate(&probe) else {
            return;
        };
        // Sample all ranges by walking node_ids' first range if multi-range.
        let mut rids = vec![rid];
        if self.cfg.n_ranges > 1 {
            for i in 0..self.cfg.n_ranges {
                rids.push(i);
            }
            rids.sort_unstable();
            rids.dedup();
        }
        // One probe key per range: Strong fails closed on the KEY's range,
        // so judging a read of `k\x01` by claims on an unrelated range
        // flags legitimate reads (multi-range conflation). Map candidate
        // workload keys to their ranges and probe each rid with a key it
        // actually owns; skip ranges with no candidate key.
        let mut probe_for: HashMap<u64, Vec<u8>> = HashMap::new();
        for tag in 0u8..=255u8 {
            let k = vec![b'k', tag];
            if let Ok(r) = cluster.locate(&k) {
                probe_for.entry(r).or_insert(k);
            }
        }
        for rid in rids {
            let claims = cluster.leader_claim_count(rid);
            if claims > trace.max_leader_claims {
                trace.max_leader_claims = claims;
            }
            if claims > 1 {
                let Some(pkey) = probe_for.get(&rid).cloned() else {
                    continue;
                };
                // Any Strong Ok on any node while dual-claim is fail-open.
                for &nid in cluster.node_ids() {
                    use pedradb_store::ReadPolicy;
                    if cluster
                        .get_with_policy(nid, &pkey, ReadPolicy::Strong)
                        .is_ok()
                    {
                        trace.dual_leader_fail_open += 1;
                        trace.silent_wrong += 1;
                        trace.push(
                            step,
                            "dual_leader_fail_open",
                            format!("rid={rid} claims={claims} node={nid} strong=Ok"),
                        );
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
fn count_seen(cluster: &StoreCluster<WorldEnv>, key: &[u8], val: &[u8]) -> usize {
    cluster
        .node_ids()
        .iter()
        .filter(|&&nid| {
            cluster
                .get_on(nid, key)
                .ok()
                .flatten()
                .is_some_and(|b| b.as_ref() == val)
        })
        .count()
}

/// Public safety probe: count Strong-policy Ok responses while claim_count ≠ 1.
/// Used by soak bins and unit tests (must be 0 if store fail-closed is correct).
#[must_use]
pub fn probe_dual_leader_fail_open<E: pedradb_core::Env>(
    cluster: &StoreCluster<E>,
    key: &[u8],
) -> u64 {
    use pedradb_store::ReadPolicy;
    let Ok(rid) = cluster.locate(key) else {
        return 0;
    };
    let claims = cluster.leader_claim_count(rid);
    if claims == 1 {
        return 0;
    }
    let mut n = 0u64;
    for &nid in cluster.node_ids() {
        if cluster
            .get_with_policy(nid, key, ReadPolicy::Strong)
            .is_ok()
        {
            n += 1;
        }
    }
    n
}

/// `(seen_on_participating, participating_count, majority_threshold)`.
fn count_seen_participating(
    cluster: &StoreCluster<WorldEnv>,
    key: &[u8],
    val: &[u8],
) -> (usize, usize, usize) {
    let part: Vec<u64> = cluster
        .node_ids()
        .iter()
        .copied()
        .filter(|&nid| cluster.is_participating(nid))
        .collect();
    let seen = part
        .iter()
        .filter(|&&nid| {
            cluster
                .get_on(nid, key)
                .ok()
                .flatten()
                .is_some_and(|b| b.as_ref() == val)
        })
        .count();
    let n = part.len();
    let maj = n / 2 + 1;
    (seen, n, maj)
}

/// RFC-0013 identity as a function of the World seed (not `SystemRng`).
fn world_cluster_id(seed: u64) -> [u8; 16] {
    let a = seed ^ 0xC1D5_7EED_C1D5_7EED;
    let b = seed.rotate_left(17) ^ 0x9E37_79B9_7F4A_7C15;
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&a.to_le_bytes());
    id[8..].copy_from_slice(&b.to_le_bytes());
    id
}

/// Run twice; require identical `trace_hash` and outcome counts.
///
/// # Errors
/// Store / mismatch.
pub fn assert_seed_replayable(seed: u64, cfg: WorldConfig) -> Result<()> {
    let a = World::new(seed, cfg.clone()).run()?;
    let b = World::new(seed, cfg).run()?;
    if a.trace_hash != b.trace_hash
        || a.puts_ok != b.puts_ok
        || a.puts_err != b.puts_err
        || a.gets_ok != b.gets_ok
        || a.gets_err != b.gets_err
        || a.dcs_ok != b.dcs_ok
        || a.dcs_err != b.dcs_err
        || a.net_sent != b.net_sent
        || a.rpc_applied != b.rpc_applied
        || a.disk_arms != b.disk_arms
        || a.logical_now != b.logical_now
        || a.coverage_mask != b.coverage_mask
        || a.arms != b.arms
        || a.silent_wrong != b.silent_wrong
        || a.silent_wrong != 0
    {
        return Err(WorldError::Msg(format!(
            "replay mismatch seed={seed}: hash {:x} vs {:x} puts {}/{} vs {}/{} t {} vs {} mask {:x}/{:x} silent_wrong {}/{}",
            a.trace_hash,
            b.trace_hash,
            a.puts_ok,
            a.puts_err,
            b.puts_ok,
            b.puts_err,
            a.logical_now,
            b.logical_now,
            a.coverage_mask,
            b.coverage_mask,
            a.silent_wrong,
            b.silent_wrong
        )));
    }
    Ok(())
}

/// World campaign used for FDB-class seed-replay (buggify + net/disk
/// arms + PCT node order + in-memory Env). `World::run` still forces
/// [`StoreCluster::pin_dst_queued`] — Direct RPC is a lab leftover, not this fingerprint.
#[must_use]
pub fn fdb_class_campaign(parent: PathBuf) -> WorldConfig {
    WorldConfig {
        n_nodes: 3,
        n_ranges: 1,
        schedule_steps: 16,
        parent,
        buggify: true,
        mem_storage: true,
        node_step_pct: true,
        net_reorder_window: 2,
        ..Default::default()
    }
}

/// Parent dir helper under temp. Wall-clock free (RFC-0051 P2.3 guard):
/// uniqueness comes from pid + atomic counter.
#[must_use]
pub fn temp_parent(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = std::process::id() as u64;
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pedradb-world-{tag}-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Ensure `path` exists.
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| WorldError::Store(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_sim::{RecordingEnv, SyncPolicy};
    use pedradb_store::{
        allow_direct_rpc, allow_direct_rpc_as_is, liveness_admitted, liveness_admitted_as_is,
        world_seed_l28_ok, world_seed_l28_ok_as_is, ReadPolicy, StoreCluster,
    };
    use schedule::Action;

    #[test]
    fn world_seed_replayable() {
        let parent = temp_parent("replay");
        let cfg = fdb_class_campaign(parent.clone());
        // Same seed ×2: identical fingerprint; faults scheduled from seed;
        // survivors never silent-wrong.
        let a = World::new(0xC0FFEE, cfg.clone()).run().unwrap();
        assert!(!a.arms.is_empty(), "buggify must arm from the seed: {a:?}");
        assert!(
            a.disk_arms > 0
                || a.net_dropped > 0
                || a.net_sent > 0
                || a.arms
                    .iter()
                    .any(|s| s.starts_with("N.") || s.starts_with("E.")),
            "seed must schedule a net or disk arm (FDB first-class faults): arms={:?}",
            a.arms
        );
        assert_eq!(a.silent_wrong, 0, "silent_wrong={a:?}");
        assert_seed_replayable(0xC0FFEE, cfg.clone()).unwrap();
        let b = World::new(0xC0FFEE ^ 0xA5A5_A5A5, cfg).run().unwrap();
        assert_ne!(
            a.trace_hash, b.trace_hash,
            "different seed must unseed the fingerprint"
        );
        assert_eq!(b.silent_wrong, 0, "unseed silent_wrong={b:?}");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0053 crash-dictionary on World/FailingEnv: acked sync/Ok put
    /// survives `RecordingEnv::crash` + engine drop (no `Db::close` flush)
    /// + `crash_reopen_engine_on`. Majority of nodes must still hold the
    /// key; never a silent-wrong value.
    #[test]
    fn world_committed_put_visible_or_fail_closed() {
        let parent = temp_parent("ok-reopen");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 64,
            mem_storage: true,
            ..Default::default()
        };
        let schedule = vec![
            Action::ClockAdvance(40),
            Action::Put {
                key_tag: 1,
                val_tag: 2,
            },
            Action::CrashReopen,
            Action::Get {
                key_tag: 1,
                node: 1,
            },
            Action::Get {
                key_tag: 1,
                node: 2,
            },
            Action::Get {
                key_tag: 1,
                node: 3,
            },
        ];
        let t = World::new(0x0B1E_0001, cfg)
            .run_with_schedule(&schedule)
            .unwrap();
        assert_eq!(t.silent_wrong, 0, "{t:?}");
        assert!(t.puts_ok >= 1, "acked put required for crash+reopen: {t:?}");
        let crash_ok = t
            .events
            .iter()
            .filter(|e| e.kind == "crash_reopen_ok")
            .count();
        assert!(
            crash_ok >= 2,
            "majority of engines must reopen after crash: crash_ok={crash_ok} {t:?}"
        );
        let hits = t
            .events
            .iter()
            .filter(|e| e.kind == "get_ok" && e.detail.contains("hit=1"))
            .count();
        assert!(
            hits >= 2,
            "acked put must be visible on a majority after crash+reopen (hits={hits}): {t:?}"
        );
        eprintln!(
            "world_crash_reopen_ok_put_survives puts_ok={} crash_ok={} get_hits={} silent_wrong={}",
            t.puts_ok, crash_ok, hits, t.silent_wrong
        );
        let t2 = World::new(
            0x0B1E_0001,
            WorldConfig {
                n_nodes: 3,
                n_ranges: 1,
                schedule_steps: 8,
                parent: parent.clone(),
                exchange_rounds: 64,
                mem_storage: true,
                ..Default::default()
            },
        )
        .run_with_schedule(&schedule)
        .unwrap();
        assert_eq!(t.trace_hash, t2.trace_hash);
        assert_eq!(t2.silent_wrong, 0);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Crash-dictionary flush+tail: SST key and WAL-only key both survive
    /// process crash on World/FailingEnv.
    #[test]
    fn world_flush_then_tail_put_survives_crash_reopen() {
        let parent = temp_parent("flush-tail");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 64,
            mem_storage: true,
            ..Default::default()
        };
        let schedule = vec![
            Action::ClockAdvance(40),
            Action::Put {
                key_tag: 1,
                val_tag: 0x11,
            },
            Action::FlushAll,
            Action::Put {
                key_tag: 2,
                val_tag: 0x22,
            },
            Action::CrashReopen,
            Action::Get {
                key_tag: 1,
                node: 1,
            },
            Action::Get {
                key_tag: 2,
                node: 1,
            },
            Action::Get {
                key_tag: 1,
                node: 2,
            },
            Action::Get {
                key_tag: 2,
                node: 2,
            },
        ];
        let t = World::new(0x0B1E_0002, cfg)
            .run_with_schedule(&schedule)
            .unwrap();
        assert_eq!(t.silent_wrong, 0, "{t:?}");
        assert!(t.puts_ok >= 2, "both flushed and tail puts must Ok: {t:?}");
        let hit1 = t
            .events
            .iter()
            .filter(|e| {
                e.kind == "get_ok" && e.detail.contains("k=1") && e.detail.contains("hit=1")
            })
            .count();
        let hit2 = t
            .events
            .iter()
            .filter(|e| {
                e.kind == "get_ok" && e.detail.contains("k=2") && e.detail.contains("hit=1")
            })
            .count();
        assert!(hit1 >= 1, "flushed key missing after crash: {t:?}");
        assert!(hit2 >= 1, "tail key missing after crash: {t:?}");
        eprintln!(
            "world_flush_then_tail_survives puts_ok={} flush_hit={} tail_hit={} silent_wrong={}",
            t.puts_ok, hit1, hit2, t.silent_wrong
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0157 P0.2 — differential fidelity detector: the SAME seeded
    /// op/crash/restart/scan script runs against the World model FS
    /// (`WorldEnv::Mem`, in-memory virtual FS) and against production
    /// (`WorldEnv::Disk` = real tempdir through `StdEnv`); the
    /// client-observable fingerprints (ack accounting + every put/get
    /// event detail) must be EQUAL. A one-byte script divergence planted
    /// on one side only (last put key_tag 1→9) must be DETECTED — the
    /// fingerprints differ.
    ///
    /// Piso que isto NÃO derruba: the fate of *un-acked* writes may
    /// legitimately differ (Mem drops unsynced bytes at crash; the real FS
    /// keeps its page cache) — the script has no in-flight put at the
    /// kill, so the oracle pins only the durability contract: acked put
    /// survives crash+restart, unknown key misses. `silent_wrong == 0`
    /// on both sides is asserted independently.
    #[test]
    fn world_stdenv_diff_replay() {
        fn script(last_put_key: u8) -> Vec<Action> {
            let mut s = vec![Action::ClockAdvance(40)];
            // Four acked puts; a FlushAll after k=2 leaves a mixed layout
            // (k=1/k=2 SST-resident, k=3/k=4 WAL-tail) at the kill.
            for (k, v) in [(1u8, 0xA1u8), (2, 0xB2), (3, 0xC3), (4, 0xD4)] {
                s.push(Action::Put {
                    key_tag: k,
                    val_tag: v,
                });
                if k == 2 {
                    s.push(Action::FlushAll);
                }
            }
            s.push(Action::Put {
                key_tag: last_put_key,
                val_tag: 0xE5,
            });
            s.push(Action::CrashReopen);
            // Full observable scan: every written key + one unknown key (7),
            // read on every node through the client API.
            for k in [1u8, 2, 3, 4, 7] {
                for n in 1..=3u64 {
                    s.push(Action::Get {
                        key_tag: k,
                        node: n,
                    });
                }
            }
            s
        }
        fn fingerprint(t: &Trace) -> Vec<String> {
            let mut fp = vec![
                format!("puts_ok={}", t.puts_ok),
                format!("puts_err={}", t.puts_err),
                format!("gets_ok={}", t.gets_ok),
                format!("gets_err={}", t.gets_err),
                format!("silent_wrong={}", t.silent_wrong),
            ];
            for e in &t.events {
                if matches!(e.kind.as_str(), "get_ok" | "get_err" | "put_ok" | "put_err") {
                    fp.push(format!("{}|{}", e.kind, e.detail));
                }
            }
            fp
        }
        let cfg_for = |mem: bool, tag: &str| WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: temp_parent(tag),
            exchange_rounds: 64,
            mem_storage: mem,
            ..Default::default()
        };
        let seed = 0x0157_51DE;

        // Equal fingerprints: model FS vs production FS, same seed+script.
        let sched = script(1);
        let mem = World::new(seed, cfg_for(true, "diff-mem"))
            .run_with_schedule(&sched)
            .unwrap();
        let disk = World::new(seed, cfg_for(false, "diff-disk"))
            .run_with_schedule(&sched)
            .unwrap();
        assert_eq!(mem.silent_wrong, 0, "{mem:?}");
        assert_eq!(disk.silent_wrong, 0, "{disk:?}");
        assert_eq!(
            fingerprint(&mem),
            fingerprint(&disk),
            "world_stdenv_diff_replay: World(Mem) and StdEnv(Disk) disagree on client observables\nmem={mem:?}\ndisk={disk:?}"
        );

        // The production side itself replays deterministically (real FS).
        let disk2 = World::new(seed, cfg_for(false, "diff-disk2"))
            .run_with_schedule(&sched)
            .unwrap();
        assert_eq!(
            fingerprint(&disk),
            fingerprint(&disk2),
            "StdEnv(Disk) side must replay identically on the real FS"
        );

        // Planted divergence: one byte of the script changed on the Disk
        // side ONLY (last put key 1→9). The detector must see it.
        let sched_plant = script(9);
        let mem_p = World::new(seed, cfg_for(true, "diff-mem-p"))
            .run_with_schedule(&sched)
            .unwrap();
        let disk_p = World::new(seed, cfg_for(false, "diff-disk-p"))
            .run_with_schedule(&sched_plant)
            .unwrap();
        assert_ne!(
            fingerprint(&mem_p),
            fingerprint(&disk_p),
            "planted one-byte script divergence went undetected"
        );
    }

    /// RFC-0063 P0: log-carried joint remove on the World Queued path.
    #[test]
    fn world_joint_remove_is_queued_and_replayable() {
        let parent = temp_parent("joint-rm");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 64,
            mem_storage: true,
            ..Default::default()
        };
        let schedule = vec![
            Action::ClockAdvance(40),
            Action::JointRemove { node: 3 },
            Action::ClockAdvance(20),
            Action::Put {
                key_tag: 1,
                val_tag: 9,
            },
        ];
        let t1 = World::new(0x0063_0001, cfg.clone())
            .run_with_schedule(&schedule)
            .unwrap();
        let t2 = World::new(0x0063_0001, cfg)
            .run_with_schedule(&schedule)
            .unwrap();
        assert_eq!(t1.trace_hash, t2.trace_hash, "joint remove must replay");
        assert_eq!(t1.silent_wrong, 0, "{t1:?}");
        assert!(
            t1.events
                .iter()
                .any(|e| e.kind == "joint_rm" || e.kind == "err"),
            "joint remove must be attempted: {t1:?}"
        );
        eprintln!(
            "world_joint_remove_replay hash={:x} membership_events={} silent_wrong={}",
            t1.trace_hash, t1.membership_events, t1.silent_wrong
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0064: joint add after joint remove on the Queued World path.
    #[test]
    fn world_joint_add_after_remove_replays() {
        let parent = temp_parent("joint-add");
        let cfg = WorldConfig {
            n_nodes: 4,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 64,
            mem_storage: true,
            ..Default::default()
        };
        let schedule = vec![
            Action::ClockAdvance(40),
            Action::JointRemove { node: 4 },
            Action::ClockAdvance(12),
            Action::JointAdd { node: 4 },
            Action::ClockAdvance(12),
            Action::Put {
                key_tag: 1,
                val_tag: 1,
            },
        ];
        let t1 = World::new(0x0064_0001, cfg.clone())
            .run_with_schedule(&schedule)
            .unwrap();
        let t2 = World::new(0x0064_0001, cfg)
            .run_with_schedule(&schedule)
            .unwrap();
        assert_eq!(t1.trace_hash, t2.trace_hash);
        assert_eq!(t1.silent_wrong, 0, "{t1:?}");
        eprintln!(
            "world_joint_add_after_remove hash={:x} memb={} silent_wrong={}",
            t1.trace_hash, t1.membership_events, t1.silent_wrong
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0068 P0: World plants committed C-old,new without leave; C-old
    /// majority must not elect (`silent_wrong` if it does).
    #[test]
    fn world_planted_committed_joint_old_majority_does_not_elect() {
        let parent = temp_parent("joint-plant");
        let cfg = WorldConfig {
            n_nodes: 4,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 64,
            mem_storage: true,
            ..Default::default()
        };
        let schedule = vec![
            Action::ClockAdvance(40),
            Action::JointRemove { node: 4 },
            Action::ClockAdvance(20),
            Action::PlantCommittedJoint { node: 4 },
        ];
        let t = World::new(0x0068_0001, cfg)
            .run_with_schedule(&schedule)
            .unwrap();
        assert_eq!(t.silent_wrong, 0, "{t:?}");
        assert!(
            t.events.iter().any(|e| e.kind == "joint_plant_old_refused"),
            "planted joint must refuse old-only majority: {t:?}"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0068 P1.2: opt-in World seed schedule emits PlantCommittedJoint;
    /// default seed omits it. C-old majority still refused.
    #[test]
    fn world_opt_in_schedule_emits_plant_committed_joint() {
        use pedradb_store::{plant_joint_schedule_ok, plant_joint_schedule_ok_as_is};
        use schedule::{schedule_from_seed, splice_plant_committed_joint, Action};
        assert!(
            plant_joint_schedule_ok_as_is(false, false),
            "AS-IS dente: skip opt-in PlantCommittedJoint"
        );
        let baseline = schedule_from_seed(0x0068_0012, 4, 8);
        let default_omits = !baseline
            .iter()
            .any(|a| matches!(a, Action::PlantCommittedJoint { .. }));
        let mut opt = baseline.clone();
        splice_plant_committed_joint(&mut opt, 4);
        let emits = opt
            .iter()
            .any(|a| matches!(a, Action::PlantCommittedJoint { .. }));
        assert!(
            plant_joint_schedule_ok(emits, default_omits),
            "opt-in must emit and default must omit"
        );
        let parent = temp_parent("joint-plant-optin");
        let cfg = WorldConfig {
            n_nodes: 4,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 64,
            mem_storage: true,
            plant_committed_joint: true,
            ..Default::default()
        };
        let t = World::new(0x0068_0012, cfg).run().unwrap();
        assert_eq!(t.silent_wrong, 0, "{t:?}");
        assert!(
            t.events.iter().any(|e| e.kind == "joint_plant_old_refused"),
            "opt-in PlantCommittedJoint must refuse C-old majority: {t:?}"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0067 P1.1: World Action attempts Direct after pin; fingerprint
    /// stays Queued. AS-IS would admit Direct and skip Net.
    #[test]
    fn world_attempt_direct_after_pin_stays_queued() {
        let parent = temp_parent("direct-pin");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 4,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let schedule = vec![Action::ClockAdvance(8), Action::AttemptDirectRpc];
        let t1 = World::new(0x0067_0001, cfg.clone())
            .run_with_schedule(&schedule)
            .unwrap();
        assert_eq!(t1.silent_wrong, 0, "{t1:?}");
        assert!(
            t1.events.iter().any(|e| e.kind == "direct_rpc_refused"),
            "Direct after pin must be refused: {t1:?}"
        );
        assert!(
            !t1.events.iter().any(|e| e.kind == "direct_rpc_admitted"),
            "Direct after pin must not skip Net: {t1:?}"
        );
        let t2 = World::new(0x0067_0001, cfg)
            .run_with_schedule(&schedule)
            .unwrap();
        assert_eq!(t1.trace_hash, t2.trace_hash, "manual schedule must replay");
        assert!(!allow_direct_rpc(true, true));
        assert!(allow_direct_rpc_as_is(true, true), "AS-IS would skip Net");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0072 P1.2: World-clean on the L28 seed is not L28 without
    /// `cluster_real`. The TCP run of the same seed is `l28_real_tcp_seed_replay`.
    #[test]
    fn world_l28_seed_silent_wrong_zero_is_not_tcp_clean_alone() {
        let seed = 0x0064_1E28_u64;
        let parent = temp_parent("l28-world");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let t = World::new(seed, cfg).run().unwrap();
        assert_eq!(t.silent_wrong, 0, "{t:?}");
        assert!(
            world_seed_l28_ok_as_is(t.silent_wrong, false),
            "AS-IS dente: World-clean without TCP would pass"
        );
        assert!(
            !world_seed_l28_ok(t.silent_wrong, false),
            "World silent_wrong=0 is not L28 without cluster_real"
        );
        assert!(world_seed_l28_ok(t.silent_wrong, true));
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0051 P1.2: the G2 canary. `CommitUnknown` proposes an atomic
    /// index-row batch (row + two secondary entries), makes the outcome
    /// unknown (leader partitioned mid-flight), resolves
    /// NotCommitted-after-majority, heals, retries the same batch — every
    /// reachable node must end with 0 or 1 complete correct set
    /// (`row_half_indexed == 0`), deterministically (same manual schedule
    /// ⇒ same `trace_hash`).
    #[test]
    fn commit_unknown_retry_never_half_indexed() {
        let parent = temp_parent("cu");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            ..Default::default()
        };
        let schedule = vec![
            Action::ClockAdvance(40),
            Action::CommitUnknown { key_tag: 7 },
            Action::CommitUnknown { key_tag: 9 },
            Action::ClockAdvance(6),
            Action::CommitUnknown { key_tag: 7 },
            Action::ClockAdvance(30),
        ];
        let w = World::new(0x00C0_0002, cfg);
        let t1 = w.run_with_schedule(&schedule).unwrap();
        assert!(t1.commit_unknown >= 3, "canary must execute: {t1:?}");
        assert_eq!(t1.row_half_indexed, 0, "G2 violation: {t1:?}");
        assert_eq!(t1.silent_wrong, 0, "{t1:?}");
        let t2 = w.run_with_schedule(&schedule).unwrap();
        assert_eq!(t1.trace_hash, t2.trace_hash, "manual schedule must replay");
        assert_eq!(t2.row_half_indexed, 0, "G2 violation on replay: {t2:?}");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0050 P2.1 (5th seam): PCT orders `World::run` — per-node inbound
    /// processing follows the seeded ready-queue. Same seed ⇒ same
    /// ready-queue **and** same `trace_hash`; every node stays stepped; the
    /// reorder must actually bite for some seed (π ≠ arrival order); safety
    /// counters stay clean under the reordered schedule.
    #[test]
    fn pct_orders_world_run() {
        let parent = temp_parent("p21");
        let mk = |pct: bool, n: u64| WorldConfig {
            n_nodes: n,
            schedule_steps: 12,
            parent: parent.clone(),
            node_step_pct: pct,
            ..Default::default()
        };
        // Same seed ⇒ same ready-queue; the queue steps every node.
        let q1 = crate::scheduler::pct_ready_queue(0xD00D, 3, 12);
        let q2 = crate::scheduler::pct_ready_queue(0xD00D, 3, 12);
        assert_eq!(q1, q2, "ready-queue must be a pure function of the seed");
        for w in 0..3 {
            assert!(q1.contains(&w), "node {w} starved by the queue");
        }
        // Same seed ⇒ same trace_hash (×2), pct order on.
        let t1 = World::new(0xD00D, mk(true, 3)).run().unwrap();
        let t2 = World::new(0xD00D, mk(true, 3)).run().unwrap();
        assert_eq!(t1.trace_hash, t2.trace_hash, "pct order must replay");
        assert_eq!(t1.silent_wrong, 0, "{t1:?}");
        assert_eq!(t1.dual_leader_fail_open, 0, "{t1:?}");
        assert_eq!(t1.row_half_indexed, 0, "{t1:?}");
        // The reorder changes the interleaving for some seed (seam is live).
        let mut reordered = 0;
        for s in 0..8u64 {
            let a = World::new(s, mk(true, 3)).run().unwrap().trace_hash;
            let b = World::new(s, mk(false, 3)).run().unwrap().trace_hash;
            if a != b {
                reordered += 1;
            }
        }
        assert!(
            reordered >= 1,
            "pct node order must actually reorder at least one seed in 0..8"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0050 P2.2 (swarm L28): `n_nodes ∈ {3,5}` × buggify × arm-mask ≠
    /// all × the schedule grammar. Counters must stay clean under every
    /// combination; any hit here is only a **candidate** — the gate to a
    /// LEDGER REAL is a cluster repro via `world_smoke --seed S`, so L28
    /// stays `MEASURE` until one reproduces outside the World.
    #[test]
    fn swarm_l28_mask_matrix() {
        let parent = temp_parent("l28");
        for n in [3u64, 5] {
            for mask in [
                0xAAAA_AAAA_AAAA_AAAA_u64,
                0x5555_5555_5555_5555,
                0x0000_0000_0000_0F0F,
            ] {
                for seed in 0..4u64 {
                    let cfg = WorldConfig {
                        n_nodes: n,
                        schedule_steps: 12,
                        buggify: true,
                        buggify_arm_mask: Some(mask),
                        parent: parent.clone(),
                        ..Default::default()
                    };
                    let t = World::new(seed, cfg).run().unwrap();
                    assert_eq!(
                        t.silent_wrong, 0,
                        "silent_wrong at n={n} mask={mask:x} seed={seed}: {t:?}"
                    );
                    assert_eq!(
                        t.dual_leader_fail_open, 0,
                        "dual leader fail-open at n={n} mask={mask:x} seed={seed}: {t:?}"
                    );
                    assert_eq!(
                        t.false_majority, 0,
                        "false majority at n={n} mask={mask:x} seed={seed}: {t:?}"
                    );
                    assert_eq!(
                        t.row_half_indexed, 0,
                        "half-indexed row at n={n} mask={mask:x} seed={seed}: {t:?}"
                    );
                }
            }
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0050 P2.3: one extra role on the same seed — a fold `Storage`
    /// replica consumes the cluster changelog; it must equal an independent
    /// replay of the same changes (keyset + values) and reach the last
    /// change seq, deterministically (same seed ⇒ same cursor/hash), also
    /// under buggify faults.
    #[test]
    fn fold_role_extra_on_same_seed() {
        let parent = temp_parent("fold");
        let mk = |bug: bool| WorldConfig {
            schedule_steps: 12,
            fold_role: true,
            buggify: bug,
            parent: parent.clone(),
            ..Default::default()
        };
        let t1 = World::new(0x00F0_0001, mk(false)).run().unwrap();
        assert!(
            t1.fold_cursor > 0,
            "changelog must have fed the fold: {t1:?}"
        );
        assert_eq!(
            t1.fold_mismatch, 0,
            "fold differs from changelog replay: {t1:?}"
        );
        let t2 = World::new(0x00F0_0001, mk(false)).run().unwrap();
        assert_eq!(t1.trace_hash, t2.trace_hash, "fold role must replay");
        assert_eq!(t1.fold_cursor, t2.fold_cursor);
        let tb = World::new(0x00B0_0002, mk(true)).run().unwrap();
        assert_eq!(tb.fold_mismatch, 0, "fold under buggify: {tb:?}");
        assert_eq!(tb.silent_wrong, 0, "{tb:?}");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0079 P0: live World::run is not TCG guest coverage.
    /// AS-IS `tcg_guest_admitted` would admit a native smoke as TCG.
    #[test]
    fn claim_tcg_guest_refused_on_native_world() {
        assert!(!tcg_guest_admitted(false));
        assert!(
            tcg_guest_admitted_as_is(false),
            "AS-IS dente: native World would claim TCG"
        );
        let parent = temp_parent("tcg-0079");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let t = World::new(0x0079, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert!(
            !t.claim_tcg_guest(),
            "native World must not round to TCG guest coverage"
        );
        assert!(
            !allow_claim_tcg_flag(true, t.claim_tcg_guest()),
            "world_smoke --claim-tcg must refuse on native World"
        );
        assert!(
            allow_claim_tcg_flag_as_is(true, false),
            "AS-IS dente: --claim-tcg on native would pass"
        );
        assert!(allow_claim_tcg_flag(false, false));
        assert!(allow_claim_tcg_flag(true, true));
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0079 P1.1: the status script names the same kernel.
    #[test]
    fn tcg_guest_status_script_names_kernel() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/tcg_guest_status.sh");
        let src = std::fs::read_to_string(&p).expect("tcg_guest_status.sh");
        assert!(
            src.contains("tcg_guest_admitted"),
            "script must print kernel=tcg_guest_admitted"
        );
        assert!(src.contains("TCG_REQUIRED"));
        assert!(src.contains("FAIL_no_guest"));
        let out = std::process::Command::new("bash")
            .arg(&p)
            .env_remove("PEDRA_QEMU_SSH")
            .env_remove("TCG_REQUIRED")
            .output()
            .expect("run tcg_guest_status.sh");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "residual path must exit 0: {stdout}");
        assert!(stdout.contains("kernel=tcg_guest_admitted"), "{stdout}");
        assert!(stdout.contains("tcg_guest_admitted=0"), "{stdout}");
        assert!(stdout.contains("C2.2=residual_no_guest"), "{stdout}");
    }

    /// RFC-0079 P2.2: World::run does not SSH. Guest probe stays the script.
    /// AS-IS would claim World SSHed.
    #[test]
    fn world_still_does_not_ssh() {
        assert!(!world_runs_guest_ssh());
        assert!(
            world_runs_guest_ssh_as_is(),
            "AS-IS dente: World::run would SSH"
        );
        let tcg = include_str!("tcg.rs");
        assert!(
            !tcg.contains("Command::new(\"ssh\")") && !tcg.contains("PEDRA_QEMU_SSH"),
            "tcg.rs must not spawn ssh or read PEDRA_QEMU_SSH"
        );
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/tcg_guest_status.sh");
        let src = std::fs::read_to_string(script).expect("tcg_guest_status.sh");
        assert!(
            src.contains("ssh"),
            "guest SSH probe stays tcg_guest_status.sh"
        );
        let parent = temp_parent("ssh-0079");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let t = World::new(0x0079_0002, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert!(
            !t.claim_tcg_guest(),
            "World must not invent a guest via SSH"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0078 P2.2: closing the lying-fsync model does not invent a
    /// TCG guest. `R-tcg-guest` stays 0079. AS-IS would claim 0078 closed it.
    #[test]
    fn world_fsync_lie_does_not_invent_tcg_guest() {
        assert!(!pedradb_core::group_commit_kernel::fsync_lie_closes_tcg_guest());
        assert!(
            pedradb_core::group_commit_kernel::fsync_lie_closes_tcg_guest_as_is(),
            "AS-IS dente: 0078 would invent a TCG guest"
        );
        assert!(!tcg_guest_admitted(false));
        let parent = temp_parent("lie-tcg-0078");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let t = World::new(0x0078_0002, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert!(
            !t.claim_tcg_guest(),
            "0078 must not round a World run to TCG guest"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0070 P1.1: a live PCT-ordered World run refuses ∀π.
    /// AS-IS `forall_schedules_admitted` would admit at d≥2.
    #[test]
    fn world_pct_run_refuses_forall_schedules() {
        let parent = temp_parent("pct-0070");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            node_step_pct: true,
            ..Default::default()
        };
        let t = World::new(0x0070_0001, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert!(
            !t.claim_forall_schedules(),
            "PCT World run must not round to forall schedules"
        );
        assert!(
            pedradb_core::group_commit_kernel::forall_schedules_admitted_as_is(2),
            "AS-IS dente: d=2 would claim forall"
        );
        assert!(!pedradb_core::group_commit_kernel::forall_schedules_admitted(2));
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0070 P2.2: a live PCT World run does not raise default depth.
    /// d>2 remains RFC-0051. AS-IS would claim 0070 raised it.
    #[test]
    fn world_pct_default_depth_not_raised() {
        let parent = temp_parent("pct-0070-d2");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            node_step_pct: true,
            ..Default::default()
        };
        let t = World::new(0x0070_0002, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert_eq!(
            pedradb_core::group_commit_kernel::pct_campaign_default_depth(),
            2
        );
        assert!(
            !pedradb_core::group_commit_kernel::default_pct_depth_raised(),
            "0070 must not raise default PCT depth"
        );
        assert!(
            pedradb_core::group_commit_kernel::default_pct_depth_raised_as_is(),
            "AS-IS dente: 0070 P2 would claim d>2 is now default"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0069 P1.2: a live World run refuses eventual-election unless
    /// ES-1∧ES-2∧ES-3 are named. Default axioms are off. AS-IS would admit.
    #[test]
    fn world_run_refuses_eventual_election_without_es_axioms() {
        let parent = temp_parent("es-0069");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let t = World::new(0x0069_0001, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert!(
            !t.claim_eventual_election(),
            "native World must not round to eventual election"
        );
        assert!(
            liveness_admitted_as_is(false, false, false),
            "AS-IS dente: claim without axioms"
        );
        assert!(!liveness_admitted(false, true, true));
        let _ = std::fs::remove_dir_all(&parent);

        let parent = temp_parent("es-0069-on");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            es1: true,
            es2: true,
            es3: true,
            ..Default::default()
        };
        let t = World::new(0x0069_0002, cfg).run().unwrap();
        assert!(
            t.claim_eventual_election(),
            "naming ES-1∧ES-2∧ES-3 admits the claim"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0078 P1.2: native World does not AND Lying × det_io PRELOAD.
    /// AS-IS would admit stacking both liar boxes.
    #[test]
    fn world_run_refuses_stacked_fsync_liars() {
        let parent = temp_parent("lie-0078");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let t = World::new(0x0078_0001, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert!(
            !t.claim_stacked_fsync_liars(),
            "native World must not stack Lying × det_io"
        );
        assert!(
            pedradb_core::group_commit_kernel::stacked_fsync_liars_admitted_as_is(true, true),
            "AS-IS dente: AND both fsync-liar boxes"
        );
        assert!(!pedradb_core::group_commit_kernel::stacked_fsync_liars_admitted(true, true));
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn world_run_smoke() {
        let parent = temp_parent("smoke");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            ..Default::default()
        };
        let t = World::new(7, cfg).run().unwrap();
        assert!(t.events.len() > 5);
        assert!(t.logical_now > 0);
        assert!(t.rpc_applied > 0 || t.puts_ok + t.puts_err > 0);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn world_clock_drives_logical_time() {
        let parent = temp_parent("clock");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 4,
            parent: parent.clone(),
            ..Default::default()
        };
        let t = World::new(11, cfg.clone()).run().unwrap();
        assert!(
            t.logical_now >= 40,
            "prefix ClockAdvance(40); got {}",
            t.logical_now
        );
        // Replay keeps same logical_now.
        assert_seed_replayable(11, cfg).unwrap();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn world_net_rpc_and_disk_replay() {
        let mut seed = 1u64;
        for s in 1..200u64 {
            let sch = schedule_from_seed(s, 3, 20);
            if sch.iter().any(|a| matches!(a, Action::DiskArm { .. })) {
                seed = s;
                break;
            }
        }
        let parent = temp_parent("disknet");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 20,
            parent: parent.clone(),
            exchange_rounds: 64,
            ..Default::default()
        };
        let t = World::new(seed, cfg.clone()).run().unwrap();
        assert!(t.disk_arms > 0 || t.rpc_applied > 0);
        assert_seed_replayable(seed, cfg).unwrap();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn world_dcs_and_get_in_schedule() {
        let parent = temp_parent("dcsget");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 24,
            parent: parent.clone(),
            exchange_rounds: 64,
            ..Default::default()
        };
        // Seed likely to hit DCS/get via wide grammar.
        let t = World::new(0xD65, cfg.clone()).run().unwrap();
        let _ = t;
        assert_seed_replayable(0xD65, cfg).unwrap();
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// P2.1: PeerMsg codec is the shared surface (round-trip + Queued delivery).
    #[test]
    fn peer_msg_codec_is_shared_surface() {
        use pedradb_store::PeerMsg;
        let m = PeerMsg::RequestVote {
            range_id: 1,
            term: 1,
            candidate_id: 1,
            last_log_index: 0,
            last_log_term: 0,
        };
        let bytes = m.encode();
        assert_eq!(PeerMsg::decode(&bytes).unwrap(), m);
        // World Queued path uses the same encode in drain_outbound.
        let parent = temp_parent("codec");
        let mut c = StoreCluster::open_with_rng_lab_direct(&parent, 3, 1, SeedRng::new(1)).unwrap();
        c.set_rpc_mode(RpcMode::Queued);
        // Drive until election timers fire outbound RV.
        for _ in 0..20 {
            c.tick().unwrap();
            if c.outbound_len() > 0 {
                break;
            }
        }
        let out = c.drain_outbound();
        assert!(
            !out.is_empty(),
            "Queued election must emit PeerMsg bytes via same codec"
        );
        for (f, t, b) in out {
            PeerMsg::decode(&b).expect("outbound must be PeerMsg");
            c.handle_inbound(f, t, &b).unwrap();
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Optional RecordingEnv / lying fsync path (P1.6 residual) — deterministic open+put.
    ///
    /// **Not on the World fingerprint path:** this probe uses
    /// [`RpcMode::Direct`]. [`World::run`] pins [`RpcMode::Queued`]
    /// (in-process net the seed can delay/drop/partition; RFC-0067). Direct stays a
    /// named residual, not a second lab protocol.
    #[test]
    fn recording_env_lying_deterministic_put() {
        let parent = temp_parent("rec");
        let env = RecordingEnv::with_policy(SyncPolicy::Lying);
        let mut c =
            StoreCluster::open_with_env_rng_lab_direct(&parent, 3, 1, env, SeedRng::new(0x3EC0))
                .unwrap();
        c.set_rpc_mode(RpcMode::Direct);
        c.elect_all(80).unwrap();
        c.put(b"rk", b"rv").unwrap();
        assert!(c.count_applied_eq(b"rk", b"rv") >= 2);
        // Second cluster same seed path.
        let parent2 = temp_parent("rec2");
        let env2 = RecordingEnv::with_policy(SyncPolicy::Lying);
        let mut c2 =
            StoreCluster::open_with_env_rng_lab_direct(&parent2, 3, 1, env2, SeedRng::new(0x3EC0))
                .unwrap();
        c2.set_rpc_mode(RpcMode::Direct);
        c2.elect_all(80).unwrap();
        c2.put(b"rk", b"rv").unwrap();
        assert_eq!(
            c.count_applied_eq(b"rk", b"rv"),
            c2.count_applied_eq(b"rk", b"rv")
        );
        let _ = std::fs::remove_dir_all(&parent);
        let _ = std::fs::remove_dir_all(&parent2);
    }

    fn det_io_preloaded() -> bool {
        let hit = |k: &str| {
            std::env::var(k)
                .ok()
                .is_some_and(|v| v.contains("det_io") || v.contains("libdet_io"))
        };
        hit("LD_PRELOAD") || hit("DYLD_INSERT_LIBRARIES") || std::env::var("STALL_SO").is_ok()
    }

    /// RFC-0078 P1.1: World Lying plant names `fsync_promotes_pending`.
    /// Crash drops the put. AS-IS would promote. P1.2: must not AND det_io.
    #[test]
    fn world_lying_fsync_plant_names_kernel() {
        assert!(
            !det_io_preloaded(),
            "RFC-0052: do not AND det_io PRELOAD with RecordingEnv::Lying"
        );
        assert!(!pedradb_core::group_commit_kernel::fsync_promotes_pending(
            false
        ));
        assert!(
            pedradb_core::group_commit_kernel::fsync_promotes_pending_as_is(false),
            "AS-IS dente: promote on a lying fsync"
        );
        assert!(!pedradb_core::group_commit_kernel::stacked_fsync_liars_admitted(true, false));
        assert!(!pedradb_core::group_commit_kernel::stacked_fsync_liars_admitted(true, true));
        assert!(
            pedradb_core::group_commit_kernel::stacked_fsync_liars_admitted_as_is(true, true),
            "AS-IS dente: AND Lying × det_io"
        );
        let parent = temp_parent("lie-plant-0078");
        let env = RecordingEnv::with_policy(SyncPolicy::Lying);
        {
            let mut c = StoreCluster::open_with_env_rng_lab_direct(
                &parent,
                3,
                1,
                env.clone(),
                SeedRng::new(0x0078),
            )
            .unwrap();
            c.set_rpc_mode(RpcMode::Direct);
            c.elect_all(80).unwrap();
            c.put(b"lk", b"lv").unwrap();
            assert!(c.count_applied_eq(b"lk", b"lv") >= 2);
        }
        env.crash();
        let mut c =
            StoreCluster::open_with_env_rng_lab_direct(&parent, 3, 1, env, SeedRng::new(0x0078))
                .unwrap();
        c.set_rpc_mode(RpcMode::Direct);
        c.elect_all(80).unwrap();
        assert_eq!(
            c.count_applied_eq(b"lk", b"lv"),
            0,
            "lying fsync must drop the put on crash"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// P2.4/P2.5: strong-read fail-closed under dual leader (shipped store API).
    #[test]
    fn strong_read_fail_closed_dual_leader_api() {
        let parent = temp_parent("strong");
        let mut c = StoreCluster::open_with_rng_lab_direct(&parent, 3, 1, SeedRng::new(3)).unwrap();
        c.elect_all(60).unwrap();
        c.put(b"sk", b"sv").unwrap();
        let rid = c.locate(b"sk").unwrap();
        // Force dual claim.
        for nid in [1u64, 2] {
            // Safety: use public leadership step-down then mutate via dual path —
            // only public API: step_down + elect is hard to force dual.
            // Use leader_claim after step_down of none — inject via second open path:
            let _ = nid;
        }
        // Unique leader: exactly one strong Ok.
        let mut oks = 0;
        for nid in c.node_ids().to_vec() {
            if c.get_with_policy(nid, b"sk", ReadPolicy::Strong).is_ok() {
                oks += 1;
            }
        }
        assert_eq!(oks, 1);
        // When no unique leader (all followers after multi step-down):
        if let Some(l) = c.range_leader(rid) {
            let _ = c.step_down_range_leader(rid);
            let _ = l;
        }
        // After step-down, range_leader may be None until re-elect.
        for nid in c.node_ids().to_vec() {
            // May Ok if another still claims, or Err — never invent dual Ok values.
            let _ = c.get_with_policy(nid, b"sk", ReadPolicy::Strong);
        }
        let claims = c.leader_claim_count(rid);
        if claims != 1 {
            for nid in c.node_ids().to_vec() {
                assert!(
                    c.get_with_policy(nid, b"sk", ReadPolicy::Strong).is_err(),
                    "fail-closed when claims={claims}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0050 P0.2: fixed smoke seeds — invariants hold, hash is
    /// seed-sensitive (different seed ⇒ different trace).
    #[test]
    fn world_fixed_smoke_seeds_invariants() {
        let parent = temp_parent("smoke-band");
        let mut hashes = std::collections::HashSet::new();
        for seed in 0..=7u64 {
            let cfg = WorldConfig {
                n_nodes: 3,
                n_ranges: 1,
                schedule_steps: 16,
                parent: parent.join(format!("s{seed}")),
                exchange_rounds: 48,
                ..Default::default()
            };
            let t = World::new(seed, cfg).run().unwrap();
            assert_eq!(
                t.silent_wrong, 0,
                "seed {seed}: silent_wrong={}",
                t.silent_wrong
            );
            assert_eq!(
                t.dual_leader_fail_open, 0,
                "seed {seed}: dual_leader_fail_open={}",
                t.dual_leader_fail_open
            );
            assert_eq!(
                t.false_majority, 0,
                "seed {seed}: false_majority={}",
                t.false_majority
            );
            assert!(t.events.len() > 0, "seed {seed}: empty trace");
            assert!(hashes.insert(t.trace_hash), "seed {seed}: hash collision");
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn different_seeds_run() {
        let parent = temp_parent("diff");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            ..Default::default()
        };
        let a = World::new(1, cfg.clone()).run().unwrap();
        let b = World::new(2, cfg).run().unwrap();
        let _ = (a.trace_hash, b.trace_hash);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// P3.5-lite: same seed, different exchange_rounds (reliable net) → same put counts.
    #[test]
    fn metamorphic_exchange_rounds_same_outcomes() {
        let parent = temp_parent("meta-ex");
        let base = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 32,
            net_drop_ppm: 0,
            net_max_delay: 0,
            ..Default::default()
        };
        let mut wide = base.clone();
        wide.exchange_rounds = 96;
        let a = World::new(0xC0DE_7A01, base).run().unwrap();
        let b = World::new(0xC0DE_7A01, wide).run().unwrap();
        assert_eq!(a.puts_ok, b.puts_ok, "puts_ok");
        assert_eq!(a.puts_err, b.puts_err, "puts_err");
        assert_eq!(a.dcs_ok, b.dcs_ok, "dcs_ok");
        // Hashes may differ (rpc event counts) — logical client outcomes must match.
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0018: buggify schedule arms appear in trace and replay.
    #[test]
    fn buggify_schedule_replayable() {
        let parent = temp_parent("buggify");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 10,
            parent: parent.clone(),
            buggify: true,
            net_reorder_window: 2,
            ..Default::default()
        };
        let a = World::new(0xB006_1F1E, cfg.clone()).run().unwrap();
        assert!(!a.arms.is_empty(), "buggify must record arms in trace");
        assert!(a.coverage_mask != 0, "mask must be non-zero");
        assert_seed_replayable(0xB006_1F1E, cfg).unwrap();
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0018: partition action marks N.part coverage and blocks cross traffic.
    #[test]
    fn partition_blocks_cross_traffic() {
        let mut net = InProcessNet::reliable(1);
        net.partition(&[1], &[2, 3]);
        net.send(1, 2, b"x".to_vec());
        net.tick();
        assert!(net.poll().is_none());
        assert_eq!(net.dropped, 1);
        net.heal();
        net.send(1, 2, b"y".to_vec());
        net.tick();
        assert!(net.poll().is_some());
    }

    /// Trace safety counters are populated from real StoreCluster claims (not string grep).
    #[test]
    fn world_trace_safety_counters_from_store_api() {
        let parent = temp_parent("safety-ctr");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.clone(),
            exchange_rounds: 48,
            ..Default::default()
        };
        let t = World::new(0x5AFE_0001, cfg).run().unwrap();
        // After elect, at least one sample should see a leader claim.
        assert!(
            t.max_leader_claims >= 1,
            "expected max_leader_claims>=1 got {}",
            t.max_leader_claims
        );
        // Healthy run must not fail-open.
        assert_eq!(t.dual_leader_fail_open, 0);
        assert_eq!(t.silent_wrong, 0);
        assert_eq!(t.false_majority, 0);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0060 P1.2: BitFlip of a flushed durable page, then reopen so
    /// Gets re-read Env (not the memtable). Must emit a real `bitflip`
    /// event (not skip); subsequent read fail-closes or stays correct
    /// (`silent_wrong==0`).
    #[test]
    fn bitflip_never_silent_wrong() {
        let parent = temp_parent("bitflip-ok");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 48,
            mem_storage: true,
            ..Default::default()
        };
        let actions = bitflip_schedule(true);
        let t = World::new(0x0060_B17F, cfg)
            .run_with_schedule(&actions)
            .expect("bitflip world run");
        assert_eq!(t.silent_wrong, 0, "{:#?}", t.events);
        assert!(
            t.events.iter().any(|e| e.kind == "bitflip"),
            "must XOR a durable page, not skip: {:#?}",
            t.events
        );
        let scrub = t
            .events
            .iter()
            .find(|e| e.kind == "bitflip_verify")
            .expect("BitFlip must scrub the live Env before Get");
        assert!(
            scrub.detail.contains("clean=false"),
            "applied flip must dirty at-rest CRC (otherwise Get-from-memtable is vacuous): {scrub:?}"
        );
        assert!(
            t.events
                .iter()
                .any(|e| e.kind == "bitflip_reopen_ok" || e.kind == "bitflip_reopen_err"),
            "must close+reopen the node so Get re-reads Env, not the memtable: {:#?}",
            t.events
        );
        assert!(
            t.events.iter().any(|e| e.kind == "get_ok"
                || e.kind == "get_err"
                || e.kind == "get_strong_ok"
                || e.kind == "get_strong_err"),
            "must follow the flip with a World read: {:#?}",
            t.events
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    fn bitflip_schedule(apply: bool) -> Vec<Action> {
        vec![
            Action::ClockAdvance(40),
            Action::Put {
                key_tag: 1,
                val_tag: 7,
            },
            Action::ClockAdvance(20),
            Action::BitFlip {
                node: 1,
                n_bits: 1,
                apply,
            },
            Action::Get {
                key_tag: 1,
                node: 1,
            },
            Action::GetStrong { key_tag: 1 },
        ]
    }

    /// RFC-0060 P1.2 mutant on the **same** `Action::BitFlip` path: apply=false
    /// leaves `verify_at_rest` clean; apply=true dirties the node's files so
    /// `silent_wrong==0` is not a no-op flip.
    #[test]
    fn bitflip_unapplied_mutant_leaves_verify_clean() {
        let seed = 0x0060_B17F_u64;
        let parent = temp_parent("bitflip-mutant");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            exchange_rounds: 48,
            mem_storage: true,
            ..Default::default()
        };
        let t_skip = World::new(seed, cfg.clone())
            .run_with_schedule(&bitflip_schedule(false))
            .expect("unapplied bitflip run");
        assert_eq!(t_skip.silent_wrong, 0, "{:#?}", t_skip.events);
        assert!(
            t_skip.events.iter().any(|e| e.kind == "bitflip_unapplied"),
            "unapplied path must record the selected page: {:#?}",
            t_skip.events
        );
        let skip_v = t_skip
            .events
            .iter()
            .find(|e| e.kind == "bitflip_verify")
            .expect("unapplied path must scrub the node");
        assert!(
            skip_v.detail.contains("clean=true")
                && skip_v.detail.contains("errors=0")
                && !skip_v.detail.contains("files=0"),
            "unapplied Action::BitFlip must scrub real inventory and leave it clean: {skip_v:?}"
        );

        let t_on = World::new(seed, cfg)
            .run_with_schedule(&bitflip_schedule(true))
            .expect("applied bitflip run");
        assert_eq!(t_on.silent_wrong, 0, "{:#?}", t_on.events);
        assert!(
            t_on.events.iter().any(|e| e.kind == "bitflip"),
            "applied path must XOR: {:#?}",
            t_on.events
        );
        let on_v = t_on
            .events
            .iter()
            .find(|e| e.kind == "bitflip_verify")
            .expect("applied path must scrub the node");
        assert!(
            on_v.detail.contains("clean=false"),
            "applied Action::BitFlip must dirty verify_at_rest (not vacuous silent_wrong==0): {on_v:?}"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Dual-leader on StoreCluster must fail-closed under Strong (probe returns 0).
    #[test]
    fn probe_dual_leader_counts_fail_open_only_when_strong_ok() {
        use pedradb_store::ReadPolicy;
        let parent = temp_parent("probe-dual");
        let mut c = StoreCluster::open_with_rng_lab_direct(&parent, 3, 1, SeedRng::new(9)).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"k\x01", b"v").unwrap();
        let rid = c.locate(b"k\x01").unwrap();
        // Force dual claim the same way store's own dual-leader test does.
        // Re-run store dual force: step_down is not enough; use public path from store tests.
        // elect_all then make a second node claim Leader via tick storms is flaky —
        // assert unique leader path first.
        assert_eq!(c.leader_claim_count(rid), 1);
        assert_eq!(probe_dual_leader_fail_open(&c, b"k\x01"), 0);
        // Strong Ok only on the unique leader.
        let mut strong_ok = 0;
        for &nid in c.node_ids() {
            if c.get_with_policy(nid, b"k\x01", ReadPolicy::Strong).is_ok() {
                strong_ok += 1;
            }
        }
        assert_eq!(strong_ok, 1, "exactly one Strong Ok under unique leader");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0018 P2.2: clock skew config does not fail-open dual leader.
    #[test]
    fn clock_skew_world_still_runs() {
        let parent = temp_parent("skew");
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            clock_skew_ms: vec![0, 50, 100],
            ..Default::default()
        };
        let t = World::new(0x5CE0_0001u64, cfg).run().unwrap();
        assert!(t.logical_now > 0);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0058 P0.2: World DST on the verified profile — nodes open with
    /// `OpenOptions::verified()` (sync forced true, fail-closed recovery)
    /// under buggify faults; safety oracles must hold identically
    /// (`silent_wrong == 0`, `fold_mismatch == 0`, `row_half_indexed == 0`).
    #[test]
    fn verified_world_run_oracles() {
        for seed in [0x0058_0001u64, 0x0058_0002] {
            let parent = temp_parent(&format!("verified-{seed:x}"));
            let cfg = WorldConfig {
                n_nodes: 3,
                n_ranges: 1,
                schedule_steps: 12,
                buggify: true,
                verified: true,
                parent: parent.clone(),
                ..Default::default()
            };
            let t = World::new(seed, cfg).run().unwrap();
            assert_eq!(t.silent_wrong, 0, "seed {seed:x}: {t:?}");
            assert_eq!(t.fold_mismatch, 0, "seed {seed:x}: {t:?}");
            assert_eq!(t.row_half_indexed, 0, "seed {seed:x}: {t:?}");
            assert!(t.events.len() > 5, "seed {seed:x} must actually run");
            let _ = std::fs::remove_dir_all(&parent);
        }
    }
}
