//! L28 REAL-cluster durability gate (RFC-0072 / R-swarm-real).
//!
//! An acked put is L28-clean only if get, get-after-kill, and get-after-restart
//! all succeed. Production [`crate`] `cluster_real` calls [`l28_durability_ok`].

#![forbid(unsafe_code)]

//! **Single artifact (Aeneas-paid):** this file is what `rustc` links and
//! what the Lean theorems run over — Charon+Aeneas extract of these exact
//! bodies. No Verus twin stands in for them.
//!
//!   ./scripts/aeneas_l28.sh




/// Durability fingerprint for a REAL 3-process TCP cluster (RFC-0072).
#[must_use]
pub fn l28_durability_ok(get_ok: bool, after_kill_ok: bool, restart_ok: bool) -> bool {
    get_ok && after_kill_ok && restart_ok
}

/// AS-IS: first get is enough (the 0072 hole — ignore kill/restart).
#[must_use]
pub fn l28_durability_ok_as_is(get_ok: bool, _after_kill_ok: bool, _restart_ok: bool) -> bool {
    get_ok
}

/// RFC-0072 P2.2: leader-kill path is the same durability triple.
/// Named so `cluster_real --leader-kill` cannot skip the kernel.
#[must_use]
pub fn l28_leader_kill_ok(get_ok: bool, after_kill_ok: bool, restart_ok: bool) -> bool {
    l28_durability_ok(get_ok, after_kill_ok, restart_ok)
}

/// AS-IS: ignore the leader-kill flag and kill+restart (get-only).
#[must_use]
pub fn l28_leader_kill_ok_as_is(get_ok: bool, _after_kill_ok: bool, _restart_ok: bool) -> bool {
    get_ok
}

/// RFC-0072 P1.2: a World-clean seed is not L28-clean unless `cluster_real`
/// durability also holds. World `silent_wrong == 0` alone is the hole.
#[must_use]
pub fn world_seed_l28_ok(world_silent_wrong: u64, cluster_real_ok: bool) -> bool {
    world_silent_wrong == 0 && cluster_real_ok
}

/// AS-IS: World-clean is enough (the 0072 P1.2 hole — skip REAL TCP).
#[must_use]
pub fn world_seed_l28_ok_as_is(world_silent_wrong: u64, _cluster_real_ok: bool) -> bool {
    world_silent_wrong == 0
}

/// RFC-0118: REAL TCP `leave_joint` was invoked (`client_leave_joint` Ok).
#[must_use]
pub fn l28_tcp_leave_ok(tcp_ok: bool) -> bool {
    tcp_ok
}

/// AS-IS: skip the TCP leave flag (the 0117 leftover — wire unused).
#[must_use]
pub fn l28_tcp_leave_ok_as_is(_tcp_ok: bool) -> bool {
    true
}

/// RFC-0121: REAL TCP planted a joint remove **and** leave was invoked.
#[must_use]
pub fn l28_tcp_plant_ok(remove_ok: bool, leave_ok: bool) -> bool {
    remove_ok && leave_ok
}

/// AS-IS: skip the plant flag (the 0120 leftover — wire unused by cluster_real).
#[must_use]
pub fn l28_tcp_plant_ok_as_is(_remove_ok: bool, _leave_ok: bool) -> bool {
    true
}

/// RFC-0121 P1.2 / 0066 P2.2: on-disk C-new-only after REAL TCP plant.
///
/// `left` is true when a recovered raft log has `MembershipJoint` with
/// `old == new`, or durable membership is already C-new and no still-active
/// joint remains (leave applied, then compacted).
#[must_use]
pub fn l28_tcp_left_ok(left: bool) -> bool {
    left
}

/// AS-IS: plant invoke is enough (the 0121 leftover — skip the on-disk scan).
#[must_use]
pub fn l28_tcp_left_ok_as_is(_left: bool) -> bool {
    true
}

/// RFC-0126 P1.2: on-disk high-water after REAL TCP plant is still above
/// the live set (3-node history survives process death after shrink to 2).
#[must_use]
pub fn l28_tcp_hw_ok(kept: bool) -> bool {
    kept
}

/// AS-IS: skip the on-disk high-water scan (the 0125 leftover — TCP restart).
#[must_use]
pub fn l28_tcp_hw_ok_as_is(_kept: bool) -> bool {
    true
}

/// RFC-0128 P1.2: after REAL TCP plant, a removed voter is not participating
/// (stale CLI/`nodes` map must not count it).
#[must_use]
pub fn l28_tcp_part_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip the participating scan (the 0127 leftover — reopen flag only).
#[must_use]
pub fn l28_tcp_part_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0130 P1.2: after REAL TCP plant + process death, recover apply
/// closes `commit > applied` (production TCP ctor).
#[must_use]
pub fn l28_tcp_apply_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip recover apply (the 0129 leftover — committed joint stays C-old).
#[must_use]
pub fn l28_tcp_apply_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0131 P1.2: after REAL TCP plant + process death, recover apply
/// closes `commit > applied` on a replica already dropped from `ids`.
#[must_use]
pub fn l28_tcp_napply_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip removed-replica recover apply (the 0130 leftover — ids only).
#[must_use]
pub fn l28_tcp_napply_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0155 P0: harness retries are not ∀ TCP traces. Always false.
#[must_use]
pub fn l28_tcp_napply_retry_admitted(_attempts: u64, _napply_ok: bool) -> bool {
    false
}

/// AS-IS: a successful napply after ≥1 attempt is rounded to ∀ TCP.
#[must_use]
pub fn l28_tcp_napply_retry_admitted_as_is(attempts: u64, napply_ok: bool) -> bool {
    attempts >= 1 && napply_ok
}

/// RFC-0132 P1.2: after REAL TCP plant + process death, recover truncate
/// persists so disk has no `index > commit` on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_trunc_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip removed-replica truncate persist (the 0131 leftover — ids only).
#[must_use]
pub fn l28_tcp_trunc_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0133 P1.2: after REAL TCP plant + process death, recover truncate
/// deletes `log_entry_key` rows past the new hi on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_odrop_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip orphan-segment drop (the 0132 leftover — watermark only).
#[must_use]
pub fn l28_tcp_odrop_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0134 P1.2: after REAL TCP plant + process death, recover abort
/// deletes leftover 2PC intents on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_abort_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip leftover abort (the 0133 leftover — ids only).
#[must_use]
pub fn l28_tcp_abort_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0135 P1.2: after REAL TCP plant + process death, persist `now_ms`
/// on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_nowms_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip now_ms persist (the 0134 leftover — ids only).
#[must_use]
pub fn l28_tcp_nowms_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0158 P2.1: on the removed replica's REAL dir, a newer-term
/// RequestVote whose hard-state persist fails must roll the term back —
/// reply, memory and disk keep the previous term (F125/F127).
#[must_use]
pub fn l28_tcp_dterm_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: keep the undurable raise (memory term above disk hard state).
#[must_use]
pub fn l28_tcp_dterm_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0136 P1.2: after REAL TCP plant + process death, persist SI hist
/// on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_hist_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip SI hist persist (the 0135 leftover — ids only).
#[must_use]
pub fn l28_tcp_hist_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0137 P1.2: after REAL TCP plant + process death, persist abort fence
/// on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_fence_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip abort-fence persist (the 0136 leftover — ids only).
#[must_use]
pub fn l28_tcp_fence_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0138 P1.2: after REAL TCP plant + process death, force-local TX
/// clear drops stuck intents on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_clear_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip force-local clear (the 0137 leftover — ids only).
#[must_use]
pub fn l28_tcp_clear_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0139 P1.2: after REAL TCP plant + process death, drop TX preimages
/// on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_pre_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip drop-preimages (the 0138 leftover — ids only).
#[must_use]
pub fn l28_tcp_pre_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0140 P1.2: after REAL TCP plant + process death, TCP ctor
/// election timeout follows disk C-new, not stale CLI.
#[must_use]
pub fn l28_tcp_peer_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip TCP disk-peer timeout (the 0139 leftover — CLI n_nodes).
#[must_use]
pub fn l28_tcp_peer_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0141 P1.2: after REAL TCP plant + process death, TCP ctor of a
/// replica dropped from `ids` must not treat HashMap first-key as identity.
#[must_use]
pub fn l28_tcp_lid_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip TCP local-id gate (the 0140 leftover — first-key always).
#[must_use]
pub fn l28_tcp_lid_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0142 P1.2: after REAL TCP plant + process death, TCP ctor must not
/// pick remote `ids.first()` as a LocalApplied reader (`empty`, not `bad node`).
#[must_use]
pub fn l28_tcp_rdr_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip TCP reader-local gate (the 0141 leftover — ids.first always).
#[must_use]
pub fn l28_tcp_rdr_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0143 P1.2: after REAL TCP plant + process death, live discard must
/// drop the uncommitted suffix on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_dsc_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip live discard (the 0142 leftover — ids only).
#[must_use]
pub fn l28_tcp_dsc_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0144 P1.2: after REAL TCP plant + process death, no-leader abort
/// persist-leader must be local so `next_index` repair runs.
#[must_use]
pub fn l28_tcp_pld_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip persist-leader locality (the 0143 leftover — ids.first).
#[must_use]
pub fn l28_tcp_pld_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0145 P1.2: after REAL TCP plant + process death, re-install of C-new
/// must step a planted Leader down on a replica dropped from `ids`.
#[must_use]
pub fn l28_tcp_std_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip step-down (the 0144 leftover — keep Role::Leader).
#[must_use]
pub fn l28_tcp_std_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0146 P1.2: after REAL TCP plant + process death, TCP ctor of a
/// remaining voter must not route `leader_hint` to the removed replica.
#[must_use]
pub fn l28_tcp_hnt_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip hint filter (the 0145 leftover — any leader_id).
#[must_use]
pub fn l28_tcp_hnt_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0147 P1.2: after REAL TCP plant + process death, TCP ctor of a
/// remaining voter must forget next/match/sent_through of the removed replica.
#[must_use]
pub fn l28_tcp_slot_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip slot drop (the 0146 leftover — keep next/match/sent_through).
#[must_use]
pub fn l28_tcp_slot_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0148 P1.2: after REAL TCP process death, TCP ctor of a remaining
/// 3-node voter must forget `sent_through` of a remote replica on oob
/// `remove_member`. 0147 joint `drop_repl_slot` is **not** this tooth.
#[must_use]
pub fn l28_tcp_sth_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip oob sent_through drop (the 0147 leftover — keep sent_through).
#[must_use]
pub fn l28_tcp_sth_ok_as_is(_ok: bool) -> bool {
    true
}

/// RFC-0068 P2.2: after REAL TCP process death, TCP ctor of a 3-node
/// voter with a planted committed C-old,new (no leave) must refuse a
/// C-old majority elect.
#[must_use]
pub fn l28_tcp_pj_ok(ok: bool) -> bool {
    ok
}

/// AS-IS: skip the planted joint (the 0148 leftover — elect on C-old).
#[must_use]
pub fn l28_tcp_pj_ok_as_is(_ok: bool) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC-0157 P1.2 — property sweep over the L28 trio
    /// (`l28_durability_ok`, `l28_leader_kill_ok`,
    /// `l28_tcp_napply_retry_admitted`): exhaustive truth tables for the
    /// boolean kernels plus a swept `attempts` space for the retry floor.
    /// Pins: the durability gate is the 3-way conjunction;
    /// leader-kill is the same triple; harness retries are never a ∀ TCP
    /// admission (always false, at every attempt count and outcome) while
    /// the AS-IS mutant rounds a retry success to ∀.
    #[test]
    fn rfc0157_property_sweep_l28_trio() {
        // Exhaustive 8-case truth tables.
        for get_ok in [false, true] {
            for after_kill_ok in [false, true] {
                for restart_ok in [false, true] {
                    let and3 = get_ok && after_kill_ok && restart_ok;
                    assert_eq!(
                        l28_durability_ok(get_ok, after_kill_ok, restart_ok),
                        and3,
                        "durability gate must be the 3-way conjunction"
                    );
                    assert_eq!(
                        l28_leader_kill_ok(get_ok, after_kill_ok, restart_ok),
                        and3,
                        "leader-kill path is the same durability triple"
                    );
                    assert_eq!(
                        l28_durability_ok_as_is(get_ok, after_kill_ok, restart_ok),
                        get_ok,
                        "AS-IS dente: first get is enough"
                    );
                }
            }
        }
        // Swept attempts space for the retry floor: never admitted.
        let attempts_sweep = [0u64, 1, 2, 3, 8, 64, u64::from(u32::MAX), u64::MAX]
            .into_iter()
            .chain((0..2_000u64).map(|i| 0x0157_8282 ^ i));
        for attempts in attempts_sweep {
            for napply_ok in [false, true] {
                assert_eq!(
                    l28_tcp_napply_retry_admitted(attempts, napply_ok),
                    false,
                    "retry after {attempts} attempts is not a ∀ TCP theorem"
                );
            }
        }
        // The AS-IS mutant keeps its tooth: a retry success is rounded
        // to ∀ TCP exactly when attempts >= 1 && napply_ok.
        for attempts in [0u64, 1, 5] {
            for napply_ok in [false, true] {
                assert_eq!(
                    l28_tcp_napply_retry_admitted_as_is(attempts, napply_ok),
                    attempts >= 1 && napply_ok
                );
            }
        }
    }

    #[test]
    fn get_only_is_not_l28_clean() {
        assert!(l28_durability_ok(true, true, true));
        assert!(!l28_durability_ok(true, false, true));
        assert!(!l28_durability_ok(true, true, false));
        assert!(!l28_durability_ok(false, true, true));
        assert!(l28_durability_ok_as_is(true, false, false));
        assert!(!l28_durability_ok_as_is(false, true, true));
    }

    fn pump_queued(c: &mut crate::StoreCluster, rounds: usize) {
        for _ in 0..rounds {
            let batch = c.drain_outbound();
            if batch.is_empty() {
                break;
            }
            for (from, to, bytes) in batch {
                c.handle_inbound(from, to, &bytes).unwrap();
            }
        }
    }

    /// Catalog three-teeth plant. Direct `get_only_is_not_l28_clean` /
    /// `l28_durability_ok_requires_after_kill_and_restart` are **not** this tooth.
    #[test]
    fn l28_durability_ok_on_live_store_is_not_ok() {
        assert!(l28_durability_ok(true, true, true));
        assert!(
            l28_durability_ok_as_is(true, false, false),
            "AS-IS dente: first get is enough"
        );
        assert!(!l28_durability_ok(true, false, false));
        let dir = std::env::temp_dir().join(format!(
            "l28-durability-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cluster =
            crate::StoreCluster::open_with_rng(&dir, 3, 1, pedradb_core::SeedRng::new(0x0152_0141))
                .unwrap();
        cluster.pin_dst_queued();
        assert_eq!(cluster.rpc_mode(), crate::RpcMode::Queued);
        for _ in 0..120 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
            if cluster.range_leader(1).is_some() {
                break;
            }
        }
        let leader = cluster
            .range_leader(1)
            .expect("Queued 3-node must elect via handle_inbound");
        let follower = (1u64..=3).find(|&id| id != leader).expect("follower");
        let key = b"rfc0152-l28".to_vec();
        let (term, last_idx, last_term) = {
            let p = cluster.nodes.get(&leader).unwrap().ranges.get(&1).unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        let put_idx = last_idx.saturating_add(1);
        let _ = cluster.drain_outbound();
        let put = crate::PeerMsg::AppendEntries {
            range_id: 1,
            term,
            leader_id: leader,
            prev_log_index: last_idx,
            prev_log_term: last_term,
            leader_commit: put_idx,
            entries: vec![crate::LogRec {
                index: put_idx,
                term,
                entry: crate::RangeEntry::Put {
                    key: key.clone(),
                    value: b"ok".to_vec(),
                    si_gen: 0,
                },
            }],
        }
        .encode();
        cluster.handle_inbound(leader, leader, &put).unwrap();
        pump_queued(&mut cluster, 64);
        for _ in 0..16 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
        }
        let get_ok = cluster.get_on(leader, &key).unwrap().as_deref() == Some(b"ok".as_ref());
        assert!(get_ok, "inbound Put must be applied on the leader");
        cluster
            .crash_reopen_engine_on(follower, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen follower");
        let after_kill_ok = cluster.count_applied_eq(&key, b"ok") >= 2;
        let restart_ok = cluster.get_on(follower, &key).unwrap().as_deref() == Some(b"ok".as_ref());
        assert!(
            l28_durability_ok(get_ok, after_kill_ok, restart_ok),
            "live cluster_real gate: get={get_ok} after={after_kill_ok} restart={restart_ok}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leader_kill_requires_named_gate() {
        assert!(l28_leader_kill_ok(true, true, true));
        assert!(!l28_leader_kill_ok(true, false, true));
        assert!(!l28_leader_kill_ok(true, true, false));
        assert!(
            l28_leader_kill_ok_as_is(true, false, false),
            "AS-IS dente: ignore leader-kill + kill/restart"
        );
        assert!(!l28_leader_kill_ok_as_is(false, true, true));
    }

    #[test]
    fn world_clean_is_not_l28_without_cluster_real() {
        assert!(world_seed_l28_ok(0, true));
        assert!(!world_seed_l28_ok(0, false));
        assert!(!world_seed_l28_ok(1, true));
        assert!(world_seed_l28_ok_as_is(0, false));
        assert!(!world_seed_l28_ok_as_is(1, true));
    }

    #[test]
    fn l28_tcp_leave_ok_requires_tcp_ok() {
        assert!(l28_tcp_leave_ok(true));
        assert!(!l28_tcp_leave_ok(false));
        assert!(l28_tcp_leave_ok_as_is(false), "AS-IS dente: skip TCP leave");
        assert!(l28_tcp_leave_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_plant_ok_requires_remove_and_leave() {
        assert!(l28_tcp_plant_ok(true, true));
        assert!(!l28_tcp_plant_ok(false, true));
        assert!(!l28_tcp_plant_ok(true, false));
        assert!(
            l28_tcp_plant_ok_as_is(false, false),
            "AS-IS dente: skip TCP plant"
        );
        assert!(l28_tcp_plant_ok_as_is(true, true));
    }

    #[test]
    fn l28_tcp_left_ok_requires_leave_on_disk() {
        assert!(l28_tcp_left_ok(true));
        assert!(!l28_tcp_left_ok(false));
        assert!(
            l28_tcp_left_ok_as_is(false),
            "AS-IS dente: skip on-disk C-new-only"
        );
        assert!(l28_tcp_left_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_hw_ok_requires_kept() {
        assert!(l28_tcp_hw_ok(true));
        assert!(!l28_tcp_hw_ok(false));
        assert!(
            l28_tcp_hw_ok_as_is(false),
            "AS-IS dente: skip on-disk high-water"
        );
        assert!(l28_tcp_hw_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_part_ok_requires_not_participating() {
        assert!(l28_tcp_part_ok(true));
        assert!(!l28_tcp_part_ok(false));
        assert!(
            l28_tcp_part_ok_as_is(false),
            "AS-IS dente: skip TCP participating"
        );
        assert!(l28_tcp_part_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_apply_ok_requires_closed() {
        assert!(l28_tcp_apply_ok(true));
        assert!(!l28_tcp_apply_ok(false));
        assert!(
            l28_tcp_apply_ok_as_is(false),
            "AS-IS dente: skip TCP recover apply"
        );
        assert!(l28_tcp_apply_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_napply_ok_requires_closed() {
        assert!(l28_tcp_napply_ok(true));
        assert!(!l28_tcp_napply_ok(false));
        assert!(
            l28_tcp_napply_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica recover apply"
        );
        assert!(l28_tcp_napply_ok_as_is(true));
    }

    /// RFC-0155 P0.3: retry-success is not ∀ TCP. AS-IS would admit.
    #[test]
    fn l28_tcp_napply_retry_admitted_is_not_forall() {
        assert!(!l28_tcp_napply_retry_admitted(1, true));
        assert!(!l28_tcp_napply_retry_admitted(3, true));
        assert!(!l28_tcp_napply_retry_admitted(0, false));
        assert!(
            l28_tcp_napply_retry_admitted_as_is(1, true),
            "AS-IS dente: one successful napply would skip retry as forall"
        );
        assert!(!l28_tcp_napply_retry_admitted_as_is(0, true));
        assert!(!l28_tcp_napply_retry_admitted_as_is(1, false));
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        for id in [
            "R-cpu",
            "R-rustc",
            "R-verus",
            "R-crc",
            "R-deps",
            "R-extract",
        ] {
            assert!(
                text.contains(&format!("\"{id}\"")),
                "never_floor must still list {id}"
            );
        }
        assert!(
            text.contains("\"db_rs_extracted\": false"),
            "glue.db_rs_extracted must stay false"
        );
    }

    #[test]
    fn l28_tcp_trunc_ok_requires_closed() {
        assert!(l28_tcp_trunc_ok(true));
        assert!(!l28_tcp_trunc_ok(false));
        assert!(
            l28_tcp_trunc_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica truncate persist"
        );
        assert!(l28_tcp_trunc_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_odrop_ok_requires_closed() {
        assert!(l28_tcp_odrop_ok(true));
        assert!(!l28_tcp_odrop_ok(false));
        assert!(
            l28_tcp_odrop_ok_as_is(false),
            "AS-IS dente: skip TCP orphan-segment drop"
        );
        assert!(l28_tcp_odrop_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_abort_ok_requires_closed() {
        assert!(l28_tcp_abort_ok(true));
        assert!(!l28_tcp_abort_ok(false));
        assert!(
            l28_tcp_abort_ok_as_is(false),
            "AS-IS dente: skip TCP leftover 2PC abort"
        );
        assert!(l28_tcp_abort_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_nowms_ok_requires_closed() {
        assert!(l28_tcp_nowms_ok(true));
        assert!(!l28_tcp_nowms_ok(false));
        assert!(
            l28_tcp_nowms_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica now_ms persist"
        );
        assert!(l28_tcp_nowms_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_dterm_ok_requires_rollback() {
        assert!(l28_tcp_dterm_ok(true));
        assert!(!l28_tcp_dterm_ok(false));
        assert!(
            l28_tcp_dterm_ok_as_is(false),
            "AS-IS dente: keep the undurable term raise"
        );
        assert!(l28_tcp_dterm_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_hist_ok_requires_closed() {
        assert!(l28_tcp_hist_ok(true));
        assert!(!l28_tcp_hist_ok(false));
        assert!(
            l28_tcp_hist_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica SI hist persist"
        );
        assert!(l28_tcp_hist_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_fence_ok_requires_closed() {
        assert!(l28_tcp_fence_ok(true));
        assert!(!l28_tcp_fence_ok(false));
        assert!(
            l28_tcp_fence_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica abort-fence persist"
        );
        assert!(l28_tcp_fence_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_clear_ok_requires_closed() {
        assert!(l28_tcp_clear_ok(true));
        assert!(!l28_tcp_clear_ok(false));
        assert!(
            l28_tcp_clear_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica force-local TX clear"
        );
        assert!(l28_tcp_clear_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_pre_ok_requires_closed() {
        assert!(l28_tcp_pre_ok(true));
        assert!(!l28_tcp_pre_ok(false));
        assert!(
            l28_tcp_pre_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica drop-preimages"
        );
        assert!(l28_tcp_pre_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_peer_ok_requires_closed() {
        assert!(l28_tcp_peer_ok(true));
        assert!(!l28_tcp_peer_ok(false));
        assert!(
            l28_tcp_peer_ok_as_is(false),
            "AS-IS dente: skip TCP disk-peer election timeout"
        );
        assert!(l28_tcp_peer_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_lid_ok_requires_closed() {
        assert!(l28_tcp_lid_ok(true));
        assert!(!l28_tcp_lid_ok(false));
        assert!(
            l28_tcp_lid_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica local-id gate"
        );
        assert!(l28_tcp_lid_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_rdr_ok_requires_closed() {
        assert!(l28_tcp_rdr_ok(true));
        assert!(!l28_tcp_rdr_ok(false));
        assert!(
            l28_tcp_rdr_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica reader-local gate"
        );
        assert!(l28_tcp_rdr_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_dsc_ok_requires_closed() {
        assert!(l28_tcp_dsc_ok(true));
        assert!(!l28_tcp_dsc_ok(false));
        assert!(
            l28_tcp_dsc_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica live discard"
        );
        assert!(l28_tcp_dsc_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_pld_ok_requires_closed() {
        assert!(l28_tcp_pld_ok(true));
        assert!(!l28_tcp_pld_ok(false));
        assert!(
            l28_tcp_pld_ok_as_is(false),
            "AS-IS dente: skip TCP persist-leader locality"
        );
        assert!(l28_tcp_pld_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_std_ok_requires_closed() {
        assert!(l28_tcp_std_ok(true));
        assert!(!l28_tcp_std_ok(false));
        assert!(
            l28_tcp_std_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica Leader step-down"
        );
        assert!(l28_tcp_std_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_hnt_ok_requires_closed() {
        assert!(l28_tcp_hnt_ok(true));
        assert!(!l28_tcp_hnt_ok(false));
        assert!(
            l28_tcp_hnt_ok_as_is(false),
            "AS-IS dente: skip TCP leader-hint membership filter"
        );
        assert!(l28_tcp_hnt_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_slot_ok_requires_closed() {
        assert!(l28_tcp_slot_ok(true));
        assert!(!l28_tcp_slot_ok(false));
        assert!(
            l28_tcp_slot_ok_as_is(false),
            "AS-IS dente: skip TCP remaining-voter repl-slot drop"
        );
        assert!(l28_tcp_slot_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_sth_ok_requires_closed() {
        assert!(l28_tcp_sth_ok(true));
        assert!(!l28_tcp_sth_ok(false));
        assert!(
            l28_tcp_sth_ok_as_is(false),
            "AS-IS dente: skip TCP remaining-voter sent_through drop"
        );
        assert!(l28_tcp_sth_ok_as_is(true));
    }

    #[test]
    fn l28_tcp_pj_ok_requires_closed() {
        assert!(l28_tcp_pj_ok(true));
        assert!(!l28_tcp_pj_ok(false));
        assert!(
            l28_tcp_pj_ok_as_is(false),
            "AS-IS dente: skip TCP planted committed-joint-without-leave"
        );
        assert!(l28_tcp_pj_ok_as_is(true));
    }

    /// RFC-0126 P2.1: REAL TCP on-disk high-water is a campaign, not ∀ traces.
    /// `R-joint` / `R-swarm-real` stay continuous.
    #[test]
    fn l28_tcp_hw_campaign_is_not_forall_traces() {
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            text.contains("\"id\": \"R-swarm-real\""),
            "R-swarm-real must stay in the residual catalog"
        );
        assert!(
            text.contains("campaign not a theorem"),
            "REAL TCP high-water must refuse forall traces"
        );
        assert!(
            text.contains("l28_tcp_hw_ok"),
            "R-joint close must name the on-disk high-water kernel"
        );
        assert!(
            text.contains("l28_real_tcp_high_water_after_remove"),
            "R-joint close must name the 3-process high-water tooth"
        );
        let catalog = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/catalog.json");
        let cat = std::fs::read_to_string(&catalog).expect("catalog.json");
        assert!(
            cat.contains("\"id\": \"l28_tcp_hw\""),
            "catalog pair l28_tcp_hw must stay"
        );
    }

    /// RFC-0121 P2.1: REAL TCP plant + on-disk C-new-only is a campaign,
    /// not ∀ traces. `R-joint` / `R-swarm-real` stay continuous.
    #[test]
    fn l28_tcp_plant_campaign_is_not_forall_traces() {
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            text.contains("\"id\": \"R-swarm-real\""),
            "R-swarm-real must stay in the residual catalog"
        );
        assert!(
            text.contains("campaign not a theorem"),
            "REAL TCP plant must refuse forall traces"
        );
        assert!(
            text.contains("l28_tcp_left_ok"),
            "R-joint close must name the on-disk leave kernel"
        );
        assert!(
            text.contains("l28_real_tcp_remove_member_left_on_disk"),
            "R-joint close must name the 3-process plant tooth"
        );
    }

    /// RFC-0118 P2.1: REAL TCP leave is a campaign, not ∀ traces.
    #[test]
    fn l28_tcp_leave_campaign_is_not_forall_traces() {
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            text.contains("\"id\": \"R-swarm-real\""),
            "R-swarm-real must stay in the residual catalog"
        );
        assert!(
            text.contains("campaign not a theorem"),
            "REAL TCP leave must refuse forall traces"
        );
    }

    /// RFC-0118 P2.2: R-verus stays never_floor (twin freeze is not a verifier).
    #[test]
    fn l28_tcp_leave_verus_still_never() {
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
    }

    /// RFC-0121 P2.2: catalog twin of `l28_tcp_left_ok` is freeze, not a
    /// verified verifier. `R-verus` stays `never_floor`.
    #[test]
    fn l28_tcp_plant_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"l28_tcp_left\""),
            "plant kernel must stay a catalog twin pair"
        );
        assert!(
            catalog.contains("\"entry\": \"l28_tcp_left_ok\""),
            "catalog entry must stay l28_tcp_left_ok"
        );
        let twin = std::fs::read_to_string(crate_root.join("src/l28.rs")).expect("src/l28.rs");
        assert!(
            twin.contains("fn l28_tcp_left_ok"),
            "kernel freeze of l28_tcp_left_ok is the term the extract pays"
        );
    }

    /// RFC-0126 P2.2: catalog twin of `l28_tcp_hw_ok` is freeze, not a
    /// verified verifier. `R-verus` stays `never_floor`.
    #[test]
    fn l28_tcp_hw_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"l28_tcp_hw\""),
            "high-water kernel must stay a catalog twin pair"
        );
        assert!(
            catalog.contains("\"entry\": \"l28_tcp_hw_ok\""),
            "catalog entry must stay l28_tcp_hw_ok"
        );
        let twin = std::fs::read_to_string(crate_root.join("src/l28.rs")).expect("src/l28.rs");
        assert!(
            twin.contains("fn l28_tcp_hw_ok"),
            "kernel freeze of l28_tcp_hw_ok is the term the extract pays"
        );
    }
}
