//! RFC-0020 P1.5 — PeerMsg decode fuzz smoke (no panic).

use pedradb_store::PeerMsg;

fn seed_msgs() -> Vec<Vec<u8>> {
    vec![
        PeerMsg::RequestVote {
            range_id: 1,
            term: 2,
            candidate_id: 3,
            last_log_index: 4,
            last_log_term: 1,
        }
        .encode(),
        PeerMsg::RequestVoteReply {
            range_id: 1,
            term: 2,
            vote_granted: true,
        }
        .encode(),
        PeerMsg::AppendEntriesReply {
            range_id: 1,
            term: 3,
            success: false,
            match_index: 0,
        }
        .encode(),
    ]
}

fn mutate(seed: &[u8], step: u64) -> Vec<u8> {
    let mut b = seed.to_vec();
    if b.is_empty() {
        return vec![(step & 0xff) as u8];
    }
    let i = (step as usize) % b.len();
    match step % 5 {
        0 => b[i] ^= 0xff,
        1 => b.truncate((i + 1).min(b.len())),
        2 => b.push(0xEE),
        3 => b.insert(i, 0x00),
        _ => b[i] = b[i].wrapping_add(7),
    }
    b
}

#[test]
fn peer_msg_fuzz_smoke_no_panic() {
    let seeds = seed_msgs();
    for (si, seed) in seeds.iter().enumerate() {
        assert!(PeerMsg::decode(seed).is_ok(), "seed {si}");
        for step in 0..128u64 {
            let m = mutate(seed, step.wrapping_add(si as u64 * 17));
            let r = std::panic::catch_unwind(|| {
                let _ = PeerMsg::decode(&m);
            });
            assert!(r.is_ok(), "PeerMsg::decode panicked si={si} step={step}");
        }
    }
}
