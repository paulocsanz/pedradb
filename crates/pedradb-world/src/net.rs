//! Pluggable network for multi-peer simulation (FDB-parity P1.1).
//!
//! Production can later sit behind the same trait (TCP codec); today
//! [`InProcessNet`] is the only impl — used by [`crate::World`] for drop /
//! reorder / partition of opaque envelopes.

use std::collections::{HashMap, HashSet, VecDeque};

use pedradb_core::{Rng, SeedRng};

/// One delivered message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// Sender peer id.
    pub from: u64,
    /// Receiver peer id.
    pub to: u64,
    /// Opaque payload (codec is the caller's problem).
    pub bytes: Vec<u8>,
}

/// Network abstraction: inject, deliver, fault.
pub trait Net {
    /// Enqueue a message from `from` to `to` (may drop/delay under policy).
    fn send(&mut self, from: u64, to: u64, bytes: Vec<u8>);

    /// Pop one ready delivery, if any (FIFO within same ready time).
    fn poll(&mut self) -> Option<Delivery>;

    /// Advance logical net time by one tick (releases delayed messages).
    fn tick(&mut self);

    /// Current logical tick.
    fn now(&self) -> u64;

    /// Bipartition: no messages cross between `left` and `right`.
    fn partition(&mut self, left: &[u64], right: &[u64]);

    /// Clear partition map (heal).
    fn heal(&mut self);

    /// Drop probability in parts-per-million for new sends (0 = reliable).
    fn set_drop_ppm(&mut self, ppm: u32);
}

#[derive(Debug, Clone)]
struct Pending {
    from: u64,
    to: u64,
    bytes: Vec<u8>,
    ready_at: u64,
}

/// In-process net with seed-driven drop/delay/reorder/corrupt and hard partitions.
#[derive(Debug, Clone)]
pub struct InProcessNet {
    rng: SeedRng,
    tick: u64,
    /// Max extra ticks of delay (0 = immediate).
    max_delay_ticks: u64,
    drop_ppm: u32,
    /// Chance per send to XOR a payload byte (CRC fail-stop at PeerMsg).
    corrupt_ppm: u32,
    /// When > 1, poll may pick a random ready message in the window (reorder).
    reorder_window: usize,
    queue: VecDeque<Pending>,
    /// If `from` and `to` are in different cells of a bipartition, drop.
    /// Encoded as peer → side (0 or 1); missing = free (all-to-all).
    side: HashMap<u64, u8>,
    /// Stats for trace.
    /// Total send attempts.
    pub sent: u64,
    /// Dropped by partition or loss policy.
    pub dropped: u64,
    /// Successfully polled deliveries.
    pub delivered: u64,
    /// Payloads corrupted before enqueue.
    pub corrupted: u64,
    /// Deliveries that were not the earliest ready index.
    pub reordered: u64,
}

impl InProcessNet {
    /// Reliable net (no drop/delay) with seed only used if policy enabled later.
    #[must_use]
    pub fn reliable(seed: u64) -> Self {
        Self {
            rng: SeedRng::new(seed ^ 0x4E45_5400),
            tick: 0,
            max_delay_ticks: 0,
            drop_ppm: 0,
            corrupt_ppm: 0,
            reorder_window: 0,
            queue: VecDeque::new(),
            side: HashMap::new(),
            sent: 0,
            dropped: 0,
            delivered: 0,
            corrupted: 0,
            reordered: 0,
        }
    }

    /// Lossy net: drop_ppm and delay up to `max_delay_ticks`.
    #[must_use]
    pub fn lossy(seed: u64, drop_ppm: u32, max_delay_ticks: u64) -> Self {
        let mut n = Self::reliable(seed);
        n.drop_ppm = drop_ppm;
        n.max_delay_ticks = max_delay_ticks;
        n
    }

    fn blocked(&self, from: u64, to: u64) -> bool {
        match (self.side.get(&from), self.side.get(&to)) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }

    /// Number of messages waiting (any ready time).
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.queue.len()
    }

    /// Set max delay ticks for new sends.
    pub fn set_max_delay(&mut self, ticks: u64) {
        self.max_delay_ticks = ticks;
    }

    /// Set corrupt probability (ppm).
    pub fn set_corrupt_ppm(&mut self, ppm: u32) {
        self.corrupt_ppm = ppm.min(1_000_000);
    }

    /// Set reorder window size (0/1 = FIFO among ready).
    pub fn set_reorder_window(&mut self, window: usize) {
        self.reorder_window = window;
    }
}

impl Net for InProcessNet {
    fn send(&mut self, from: u64, to: u64, bytes: Vec<u8>) {
        self.sent += 1;
        if self.blocked(from, to) {
            self.dropped += 1;
            return;
        }
        if self.drop_ppm > 0 {
            let r = self.rng.gen_range(1_000_000);
            if r < u64::from(self.drop_ppm) {
                self.dropped += 1;
                return;
            }
        }
        let mut bytes = bytes;
        if self.corrupt_ppm > 0 && !bytes.is_empty() {
            let r = self.rng.gen_range(1_000_000);
            if r < u64::from(self.corrupt_ppm) {
                let i = self.rng.gen_range(bytes.len() as u64) as usize;
                bytes[i] ^= 0xA5;
                self.corrupted += 1;
            }
        }
        let delay = if self.max_delay_ticks == 0 {
            0
        } else {
            self.rng.gen_range(self.max_delay_ticks + 1)
        };
        self.queue.push_back(Pending {
            from,
            to,
            bytes,
            ready_at: self.tick.saturating_add(delay),
        });
    }

    fn poll(&mut self) -> Option<Delivery> {
        let ready: Vec<usize> = self
            .queue
            .iter()
            .enumerate()
            .filter(|(_, p)| p.ready_at <= self.tick)
            .map(|(i, _)| i)
            .collect();
        if pedradb_core::write_admission_kernel::batch_is_empty(ready.len() as u64) {
            return None;
        }
        let pick = if self.reorder_window > 1 && ready.len() > 1 {
            let w = self.reorder_window.min(ready.len());
            let j = self.rng.gen_range(w as u64) as usize;
            if j != 0 {
                self.reordered += 1;
            }
            ready[j]
        } else {
            ready[0]
        };
        let p = self.queue.remove(pick).unwrap();
        if self.blocked(p.from, p.to) {
            self.dropped += 1;
            return self.poll();
        }
        self.delivered += 1;
        Some(Delivery {
            from: p.from,
            to: p.to,
            bytes: p.bytes,
        })
    }

    fn tick(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }

    fn now(&self) -> u64 {
        self.tick
    }

    fn partition(&mut self, left: &[u64], right: &[u64]) {
        self.side.clear();
        for &id in left {
            self.side.insert(id, 0);
        }
        for &id in right {
            self.side.insert(id, 1);
        }
    }

    fn heal(&mut self) {
        self.side.clear();
    }

    fn set_drop_ppm(&mut self, ppm: u32) {
        self.drop_ppm = ppm.min(1_000_000);
    }
}

/// Peers currently marked offline in a World schedule (store-level partition).
#[derive(Debug, Clone, Default)]
pub struct MembershipFault {
    offline: HashSet<u64>,
}

impl MembershipFault {
    /// Empty fault map (all online).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `id` offline or online.
    pub fn set_offline(&mut self, id: u64, offline: bool) {
        if offline {
            self.offline.insert(id);
        } else {
            self.offline.remove(&id);
        }
    }

    /// Whether `id` is marked offline.
    #[must_use]
    pub fn is_offline(&self, id: u64) -> bool {
        self.offline.contains(&id)
    }

    /// Sorted offline peer ids.
    #[must_use]
    pub fn offline_ids(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.offline.iter().copied().collect();
        v.sort_unstable();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_and_delay() {
        let mut net = InProcessNet::lossy(7, 0, 3);
        net.set_reorder_window(4);
        net.send(1, 2, b"a".to_vec());
        net.send(1, 2, b"b".to_vec());
        net.send(1, 2, b"c".to_vec());
        for _ in 0..5 {
            net.tick();
        }
        let mut n = 0;
        while net.poll().is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
        assert_eq!(net.delivered, 3);
    }

    #[test]
    fn corrupt_payload() {
        let mut net = InProcessNet::reliable(9);
        net.set_corrupt_ppm(1_000_000); // always
        net.send(1, 2, vec![0u8; 16]);
        net.tick();
        let d = net.poll().expect("delivery");
        assert_eq!(net.corrupted, 1);
        assert_ne!(d.bytes, vec![0u8; 16]);
    }

    #[test]
    fn reliable_fifo() {
        let mut n = InProcessNet::reliable(1);
        n.send(1, 2, b"a".to_vec());
        n.send(1, 2, b"b".to_vec());
        assert_eq!(n.poll().unwrap().bytes, b"a");
        assert_eq!(n.poll().unwrap().bytes, b"b");
        assert!(n.poll().is_none());
    }

    #[test]
    fn partition_drops_cross() {
        let mut n = InProcessNet::reliable(2);
        n.partition(&[1], &[2, 3]);
        n.send(1, 2, b"x".to_vec());
        assert!(n.poll().is_none());
        assert_eq!(n.dropped, 1);
        n.send(2, 3, b"y".to_vec());
        assert_eq!(n.poll().unwrap().bytes, b"y");
        n.heal();
        n.send(1, 2, b"z".to_vec());
        assert_eq!(n.poll().unwrap().bytes, b"z");
    }

    #[test]
    fn delay_needs_tick() {
        let mut n = InProcessNet::lossy(3, 0, 2);
        // Force delay by multiple sends until we see ready_at > 0 — or just tick.
        n.max_delay_ticks = 1;
        n.send(1, 2, b"d".to_vec());
        // May be ready_at 0 or 1; drain with ticks.
        let mut got = None;
        for _ in 0..4 {
            if let Some(d) = n.poll() {
                got = Some(d);
                break;
            }
            n.tick();
        }
        assert!(got.is_some());
    }
}
