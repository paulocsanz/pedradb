//! RFC-0199 P1.2 counting ladder — Rust twin of
//! `memtable_flush_amortized` (Lean:
//! `formal/aeneas/lean/FlushAmortCount.lean`, count row
//! `catalog:auto_flush_due`).
//!
//! The theorem's claim in Rust terms: a write/flush schedule whose
//! ticks consult the REAL `auto_flush_due` gate pays total flush work
//! (bytes flushed) at most equal to the bytes written plus the
//! memtable's initial fill — every byte is charged at most once, so
//! k writes under the cap amortize their flushes to O(k). The
//! schedule simulator below only counts; the fire/hold decision on
//! every tick is the real kernel's, never re-implemented.

use pedradb_core::flush_kernel::auto_flush_due;

/// One schedule step: `Write(u)` adds u bytes to the memtable, `Tick`
/// consults the real gate.
enum Step {
    Write(u64),
    Tick,
}

/// Work twin of the write/flush cycle (`run_flush_steps` in
/// `FlushAmortCount.lean`): returns (final mem bytes, total flushed
/// bytes). The ONLY policy input is the real `auto_flush_due` —
/// armed ticks fire when the kernel says so, a firing tick pays the
/// memtable's current bytes and resets it to zero.
fn run_steps(limit: u64, steps: &[Step], initial: u64) -> (u64, u64) {
    let mut mem = initial;
    let mut paid = 0u64;
    for step in steps {
        match step {
            Step::Write(u) => mem = mem.saturating_add(*u),
            Step::Tick => {
                if auto_flush_due(mem, true, limit) {
                    paid += mem;
                    mem = 0;
                }
            }
        }
    }
    (mem, paid)
}

fn write_units(steps: &[Step]) -> u64 {
    steps
        .iter()
        .map(|s| match s {
            Step::Write(u) => *u,
            Step::Tick => 0,
        })
        .sum()
}

/// The registered bound (`memtable_flush_amortized`): paid ≤ written +
/// initial — on randomized-ish adversarial schedules and the classic
/// shapes, with the real gate deciding every tick.
#[test]
fn flush_work_amortizes_to_bytes_written() {
    let schedules: Vec<(u64, Vec<Step>)> = vec![
        // cap 100, fill 90, tick holds (90 < 100), write 10, tick fires
        (
            100,
            vec![Step::Write(90), Step::Tick, Step::Write(10), Step::Tick],
        ),
        // many small writes, one flush per cap crossing
        (
            7,
            (0..50).map(|i| Step::Write(1 + (i % 3) as u64)).collect(),
        ),
        // ticks with an empty memtable never fire
        (10, vec![Step::Tick, Step::Tick, Step::Tick]),
        // write exactly to the cap: fires (>= is the kernel's edge)
        (10, vec![Step::Write(10), Step::Tick]),
        // no flush ever fires: everything stays in the final memtable
        (1000, vec![Step::Write(5), Step::Write(6), Step::Tick]),
    ];
    // pseudo-random LCG schedule over a small limit to hit ragged crossings
    let mut state = 0x2545F4914F6CDD1Du64;
    let mut ragged = Vec::new();
    for _ in 0..200 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if state >> 63 == 0 {
            ragged.push(Step::Write(1 + (state % 13)));
        } else {
            ragged.push(Step::Tick);
        }
    }
    // (limit, steps) pairs including the ragged schedule
    let mut all = schedules;
    all.push((9, ragged));

    for (limit, steps) in &all {
        for initial in [0u64, 1, 3, 40] {
            let (mem, paid) = run_steps(*limit, steps, initial);
            let written = write_units(steps);
            assert!(
                paid <= written + initial,
                "limit={limit} initial={initial}: paid {paid} > written {written} + {initial}"
            );
            // the accounting invariant (Lean: run_paid_plus_mem_le)
            assert!(paid + mem <= written + initial);
        }
    }
}

/// The firing shape: at the cap the memtable flushes and resets; under
/// the cap a tick holds and the bytes stay.
#[test]
fn firing_tick_pays_memtable_and_resets() {
    // 90 < 100: tick holds
    assert_eq!(run_steps(100, &[Step::Write(90), Step::Tick], 0), (90, 0));
    // 100 >= 100: tick fires, pays exactly the memtable bytes
    assert_eq!(run_steps(100, &[Step::Write(100), Step::Tick], 0), (0, 100));
    // 5 bytes carried in, 95 written: the flush pays the carry too —
    // and the carry is charged once (initial fill is in the bound)
    assert_eq!(run_steps(100, &[Step::Write(5), Step::Write(95), Step::Tick], 0), (0, 100));
    // 5 bytes carried in, 10 more written: 15 ≥ limit 10, so the flush
    // pays the carry too — and the carry is charged once (initial fill
    // is in the bound)
    let (mem, paid) = run_steps(10, &[Step::Write(10), Step::Tick], 5);
    assert_eq!((mem, paid), (0, 15));
}

/// The bridges (`auto_flush_due_fires_iff` /
/// `auto_flush_due_hold_under_limit`): the real gate fires exactly
/// when armed and the limit is reached — and an armed non-firing
/// check certifies the memtable is still under the limit.
#[test]
fn auto_flush_gate_fires_iff_armed_and_reached() {
    for mem in [0u64, 1, 9, 10, 11, 99, 100, 101, u64::MAX / 2] {
        for limit in [0u64, 1, 9, 10, 11, 99, 100, 101, u64::MAX] {
            for armed in [false, true] {
                let fires = auto_flush_due(mem, armed, limit);
                // fires iff armed ∧ limit ≤ mem (bridge, both directions)
                assert_eq!(fires, armed && limit <= mem);
                if !fires && armed {
                    // hold certifies mem < limit (bridge)
                    assert!(mem < limit);
                }
            }
        }
    }
    // AS-IS tooth: never fires (the degradation this credit replaces)
    assert!(!pedradb_core::flush_kernel::auto_flush_due_as_is(100, true, 10));
}
