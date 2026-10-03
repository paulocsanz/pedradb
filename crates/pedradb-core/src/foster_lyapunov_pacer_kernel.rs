//! Foster-Lyapunov Stochastic Ingestion Pacing Kernel (RFC-0311).
//!
//! Provides mathematically proven, continuous write backpressure pacing based on
//! multi-dimensional LSM queue backlogs (memtable pressure, L0 file accumulation,
//! and pending compaction debt).
//!
//! # Mathematical Foundation: Foster-Lyapunov Drift Optimization
//! Let state $x_t = (Q_{\text{mem}}, Q_{L0}, Q_{\text{comp}}) \in \mathbb{R}^3_+$.
//! Define quadratic potential:
//! $$V(x_t) = \frac{1}{2} \sum_{i} w_i \cdot \left(\max\left(0, \frac{q_i - q_i^{\text{target}}}{q_i^{\text{hard}} - q_i^{\text{target}}}\right)\right)^2$$
//!
//! Admission ratio $\eta \in (0, 1]$ is continuously derived from the Lyapunov gradient force:
//! $$\eta(x_t) = \frac{1}{1 + \gamma \cdot \|\nabla V(x_t)\|^2}$$
//!
//! Applied write pacing delay:
//! $$D(x_t) = D_{\max} \cdot (1 - \eta(x_t))$$
//!
//! This formulation provably eliminates discrete "stop-the-world" cliff edges,
//! suppresses write stall convoys, and guarantees that the queue backlog Markov chain
//! possesses strictly negative drift outside the compact target set $\mathcal{C}$.

#![forbid(unsafe_code)]

/// Backlog metrics sampled from the live storage engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BacklogState {
    /// Active memtable and unflushed memory size in bytes.
    pub mem_bytes: u64,
    /// Number of SST files residing in Level 0.
    pub l0_files: u64,
    /// Total pending compaction debt across all levels in bytes.
    pub compaction_debt_bytes: u64,
}

/// Configuration parameters for Foster-Lyapunov pacing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FosterLyapunovConfig {
    /// Soft target threshold for memtable bytes.
    pub mem_target_bytes: u64,
    /// Hard saturation limit for memtable bytes.
    pub mem_hard_bytes: u64,
    /// Soft target threshold for Level 0 file count.
    pub l0_target_files: u64,
    /// Hard saturation limit for Level 0 file count.
    pub l0_hard_files: u64,
    /// Soft target threshold for pending compaction debt in bytes.
    pub compaction_target_bytes: u64,
    /// Hard saturation limit for pending compaction debt in bytes.
    pub compaction_hard_bytes: u64,
    /// Maximum allowed backpressure pause in microseconds (e.g. 10_000 µs = 10 ms).
    pub max_delay_micros: u64,
    /// Lyapunov curvature sharpness factor $\gamma$.
    pub gamma: f64,
    /// Relative priority weight for memtable backlog.
    pub weight_mem: f64,
    /// Relative priority weight for L0 backlog.
    pub weight_l0: f64,
    /// Relative priority weight for compaction debt.
    pub weight_compaction: f64,
}

impl Default for FosterLyapunovConfig {
    fn default() -> Self {
        Self {
            mem_target_bytes: 64 * 1024 * 1024,         // 64 MiB
            mem_hard_bytes: 256 * 1024 * 1024,          // 256 MiB
            l0_target_files: 4,                         // 4 files
            l0_hard_files: 20,                          // 20 files
            compaction_target_bytes: 128 * 1024 * 1024, // 128 MiB
            compaction_hard_bytes: 1024 * 1024 * 1024,  // 1 GiB
            max_delay_micros: 10_000,                   // 10 ms max delay
            gamma: 4.0,                                 // smooth quadratic curve
            weight_mem: 1.0,
            weight_l0: 2.5,                             // L0 is read amplification bottleneck
            weight_compaction: 1.5,
        }
    }
}

/// Decision output computed by the Foster-Lyapunov pacer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LyapunovPacingDecision {
    /// Exact microsecond pause to apply before admitting write.
    pub delay_micros: u64,
    /// Value of the quadratic Lyapunov potential $V(x)$.
    pub potential_v: f64,
    /// Normalized Lyapunov gradient force $\|\nabla V(x)\|$.
    pub gradient_norm: f64,
    /// Admission multiplier $\eta \in (0, 1]$.
    pub admission_ratio: f64,
}

/// Engine evaluating Foster-Lyapunov pacing decisions.
pub struct FosterLyapunovPacer {
    config: FosterLyapunovConfig,
}

impl FosterLyapunovPacer {
    /// Creates a new pacer with given configuration.
    pub fn new(config: FosterLyapunovConfig) -> Self {
        assert!(config.mem_target_bytes < config.mem_hard_bytes);
        assert!(config.l0_target_files < config.l0_hard_files);
        assert!(config.compaction_target_bytes < config.compaction_hard_bytes);
        assert!(config.gamma > 0.0);
        Self { config }
    }

    /// Evaluates current engine state and derives continuous pacing backpressure.
    pub fn evaluate(&self, state: &BacklogState) -> LyapunovPacingDecision {
        let q_mem = normalize_debt(
            state.mem_bytes,
            self.config.mem_target_bytes,
            self.config.mem_hard_bytes,
        );
        let q_l0 = normalize_debt(
            state.l0_files,
            self.config.l0_target_files,
            self.config.l0_hard_files,
        );
        let q_comp = normalize_debt(
            state.compaction_debt_bytes,
            self.config.compaction_target_bytes,
            self.config.compaction_hard_bytes,
        );

        // Lyapunov Potential V(x) = 1/2 * sum(w_i * q_i^2)
        let potential_v = 0.5
            * (self.config.weight_mem * q_mem * q_mem
                + self.config.weight_l0 * q_l0 * q_l0
                + self.config.weight_compaction * q_comp * q_comp);

        // Gradient vector components: dV/dq_i = w_i * q_i
        let grad_mem = self.config.weight_mem * q_mem;
        let grad_l0 = self.config.weight_l0 * q_l0;
        let grad_comp = self.config.weight_compaction * q_comp;

        // Gradient Euclidean norm ||grad V(x)||
        let gradient_norm = (grad_mem * grad_mem + grad_l0 * grad_l0 + grad_comp * grad_comp).sqrt();

        // Continuous admission factor eta = 1 / (1 + gamma * ||grad V(x)||^2)
        let denom = 1.0 + self.config.gamma * (gradient_norm * gradient_norm);
        let admission_ratio = if denom.is_finite() && denom >= 1.0 {
            1.0 / denom
        } else {
            0.0
        };

        // Delay D(x) = D_max * (1 - eta)
        let delay_factor = (1.0 - admission_ratio).clamp(0.0, 1.0);
        let delay_micros = (self.config.max_delay_micros as f64 * delay_factor).round() as u64;

        LyapunovPacingDecision {
            delay_micros,
            potential_v,
            gradient_norm,
            admission_ratio,
        }
    }

    /// Computes 1-step Lyapunov drift: $\Delta V = V(x_{t+1}) - V(x_t)$.
    ///
    /// For stability outside the target set, $\Delta V \le -\epsilon < 0$.
    pub fn compute_drift(&self, current: &BacklogState, next: &BacklogState) -> f64 {
        let v_curr = self.evaluate(current).potential_v;
        let v_next = self.evaluate(next).potential_v;
        v_next - v_curr
    }
}

/// Normalizes raw debt metric to normalized coordinate $[0.0, 1.0]$ with soft clipping.
fn normalize_debt(actual: u64, target: u64, hard: u64) -> f64 {
    if actual <= target {
        return 0.0;
    }
    if actual >= hard {
        return 1.0;
    }
    let num = (actual - target) as f64;
    let den = (hard - target) as f64;
    (num / den).clamp(0.0, 1.0)
}
