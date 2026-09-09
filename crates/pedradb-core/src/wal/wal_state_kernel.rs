//! Inductive WAL state invariant (RFC-0166 P1.2): `acked ⊆ synced ⊆
//! prefixo-recuperável`.
//!
//! The live seam is [`crate::wal::writer`] + [`crate::group_commit_kernel`]
//! (fdatasync before Ok) over the [`crate::env`] barrier. This kernel is the
//! prefix geometry of one WAL segment as a state machine:
//!
//! - [`WalState::of`] builds only well-formed prefixes (acked clamped to
//!   synced, synced clamped to written);
//! - [`inv_wal`] is the inductive invariant Inv-WAL: every acked byte is
//!   barrier-durable, every durable byte is inside the log (and therefore
//!   inside every recoverable prefix — the crash ceiling of
//!   [`crate::env_crash_kernel`] is `written`);
//! - the four atoms ([`wal_append`], [`wal_sync`], [`wal_ack`],
//!   [`wal_rotate`]) each preserve Inv-WAL;
//! - [`acked_survives_every_legal_crash`] is the bridge to the crash
//!   geometry: every legal crash cut keeps at least the acked prefix.
//!
//! AS-IS mutants ack before the barrier, ack past the barrier, pretend a
//! lying sync promoted, and rotate (drop the log) with a non-durable tail —
//! each violates Inv-WAL or loses acked bytes; teeth witnesses pin all
//! holes. Verus twin: `crates/pedradb-core/verus/wal_state.rs`
//! (`scripts/verus_wal_state.sh`).

#![forbid(unsafe_code)]

use crate::env_crash_kernel::{
    crash_legal, crash_legal_as_is, sync as env_sync, CrashModel, SyncHonesty,
};

/// WAL prefix geometry: bytes acked to callers, made durable by the last
/// honest barrier, and appended to the (possibly buffered) log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalState {
    /// Bytes the caller was told `Ok` for (fdatasync before Ok).
    pub acked: u64,
    /// Bytes the last honest barrier made crash-proof (`synced <= written`).
    pub synced: u64,
    /// Bytes appended (logical length).
    pub written: u64,
}

/// Well-formed prefix: only ack what is synced, only sync what is written.
#[must_use]
pub fn wal_state_of(written: u64, synced: u64, acked: u64) -> WalState {
    let synced = synced.min(written);
    WalState {
        acked: acked.min(synced),
        synced,
        written,
    }
}

/// Inv-WAL: `acked ⊆ synced ⊆ recoverable prefix` (the legal-crash ceiling
/// is `written`, so `synced <= written` is exactly "synced is recoverable").
#[must_use]
pub fn inv_wal(s: &WalState) -> bool {
    s.acked <= s.synced && s.synced <= s.written
}

/// Append `n` bytes: the log grows; barrier and acked prefix do not move.
#[must_use]
pub fn wal_append(s: WalState, n: u64) -> WalState {
    WalState {
        written: s.written + n,
        synced: s.synced,
        acked: s.acked,
    }
}

/// Sync per honesty (env crash semantics: honest promotes every pending
/// byte; lying returns Ok and promotes nothing — RFC-0078).
#[must_use]
pub fn wal_sync(s: WalState, honesty: SyncHonesty) -> WalState {
    let m = env_sync(CrashModel::of(s.written, s.synced), honesty);
    WalState {
        written: m.written,
        synced: m.synced,
        acked: s.acked,
    }
}

/// Ack `n` appended bytes — only what the barrier already made durable.
/// Fail-closed: an ack past the barrier is refused (state unchanged).
#[must_use]
pub fn wal_ack(s: WalState, n: u64) -> WalState {
    if s.acked.saturating_add(n) <= s.synced {
        WalState {
            acked: s.acked + n,
            synced: s.synced,
            written: s.written,
        }
    } else {
        s
    }
}

/// Rotate (drop the log) — only when the whole log is durable and acked
/// (`acked == synced == written`); any non-durable tail keeps the log.
#[must_use]
pub fn wal_rotate(s: WalState) -> WalState {
    if s.acked == s.synced && s.synced == s.written {
        wal_state_of(0, 0, 0)
    } else {
        s
    }
}

/// Bridge to the crash geometry: every legal crash cut keeps at least the
/// acked prefix (floor of a legal cut is `synced >= acked`).
#[must_use]
pub fn acked_survives_every_legal_crash(s: &WalState, cut: u64) -> bool {
    !crash_legal(CrashModel::of(s.written, s.synced), cut) || cut >= s.acked
}

// --- AS-IS mutants ----------------------------------------------------------

/// AS-IS hole 1: the invariant forgets `acked ⊆ synced` ("acked" without a
/// barrier is already durability).
#[must_use]
pub fn inv_wal_as_is(s: &WalState) -> bool {
    s.synced <= s.written
}

/// AS-IS hole 2: append acks the bytes together with the write — before any
/// barrier.
#[must_use]
pub fn wal_append_as_is(s: WalState, n: u64) -> WalState {
    WalState {
        acked: s.acked + n,
        synced: s.synced,
        written: s.written + n,
    }
}

/// AS-IS hole 3: sync pretends it promoted regardless of honesty.
#[must_use]
pub fn wal_sync_as_is(s: WalState, _honesty: SyncHonesty) -> WalState {
    WalState {
        written: s.written,
        synced: s.written,
        acked: s.acked,
    }
}

/// AS-IS hole 4: ack advances past the barrier unconditionally.
#[must_use]
pub fn wal_ack_as_is(s: WalState, n: u64) -> WalState {
    WalState {
        acked: s.acked + n,
        synced: s.synced,
        written: s.written,
    }
}

/// AS-IS hole 5: rotate drops the log even with a non-durable tail.
#[must_use]
pub fn wal_rotate_as_is(_s: WalState) -> WalState {
    wal_state_of(0, 0, 0)
}

/// AS-IS hole 6: the survival corollary under the floor-less legality —
/// accepts cuts below the barrier as "survivable".
#[must_use]
pub fn acked_survives_as_is(s: &WalState, cut: u64) -> bool {
    !crash_legal_as_is(CrashModel::of(s.written, s.synced), cut) || cut >= s.acked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inv_preserved_by_every_atom() {
        // Exhaustive over the small domain: every well-formed state stays
        // well-formed under every atom, and survives every legal cut.
        for written in 0..6u64 {
            for synced in 0..=written {
                for acked in 0..=synced {
                    let s = WalState {
                        acked,
                        synced,
                        written,
                    };
                    assert!(inv_wal(&s));
                    for n in 0..4u64 {
                        let a = wal_append(s, n);
                        assert!(inv_wal(&a));
                        for h in [SyncHonesty::Honest, SyncHonesty::Lying] {
                            let y = wal_sync(a, h);
                            assert!(inv_wal(&y));
                            assert!(inv_wal(&wal_rotate(y)));
                            for cut in 0..=y.written + 2 {
                                assert!(acked_survives_every_legal_crash(&y, cut));
                            }
                        }
                        for k in 0..4u64 {
                            assert!(inv_wal(&wal_ack(a, k)));
                        }
                    }
                    assert!(inv_wal(&wal_rotate(s)));
                }
            }
        }
    }

    #[test]
    fn ack_past_barrier_refused_while_as_is_acks() {
        // synced=3, acked=2: acking 5 more needs the barrier at 7 — refused.
        let s = wal_state_of(10, 3, 2);
        assert_eq!(wal_ack(s, 5), s, "ack past the barrier is fail-closed");
        // AS-IS acks anyway: acked=7 > synced=3 breaks Inv-WAL, and the
        // survival corollary stops holding (cut=3 loses "acked" bytes).
        let bad = wal_ack_as_is(s, 5);
        assert!(inv_wal_as_is(&bad) && !inv_wal(&bad));
        assert!(!acked_survives_every_legal_crash(&bad, 3));
    }

    #[test]
    fn rotate_refuses_unsynced_tail_while_as_is_drops() {
        // acked=3=synced but 10 written: the unsynced tail keeps the log.
        let s = wal_state_of(10, 3, 3);
        assert_eq!(
            wal_rotate(s),
            s,
            "rotate with a non-durable tail is refused"
        );
        // AS-IS drops everything: the 3 acked bytes vanish from the log.
        let dropped = wal_rotate_as_is(s);
        assert_eq!(dropped, wal_state_of(0, 0, 0));
        assert!(dropped.acked < s.acked);
    }

    #[test]
    fn survival_holds_and_as_is_legality_diverges() {
        // Real corollary on a well-formed state: holds for every cut.
        let s = wal_state_of(10, 3, 3);
        for cut in 0..=12u64 {
            assert!(acked_survives_every_legal_crash(&s, cut));
        }
        // Divergence witness: cut=1 is illegal (below the barrier floor),
        // so the real corollary is vacuously true — the floor-less as-is
        // legality calls it survivable and then loses the acked prefix.
        assert!(acked_survives_every_legal_crash(&s, 1));
        assert!(!acked_survives_as_is(&s, 1));
    }

    fn freeze_pair(catalog: &[&str]) {
        let src = include_str!("wal_state_kernel.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap_or(src);
        let mut found = Vec::new();
        let mut rest = prod;
        while let Some(i) = rest.find("pub fn ") {
            rest = &rest[i + "pub fn ".len()..];
            let name = rest.split('(').next().unwrap_or("").trim();
            if catalog.contains(&name) {
                found.push(name.to_string());
            }
        }
        for n in catalog {
            assert!(
                found.iter().any(|f| f == n),
                "catalog fn {n} missing from production"
            );
        }
        assert_eq!(found.len(), catalog.len());
    }

    /// Catalog pair `wal_state`. Name avoids the bare `wal_state` substring
    /// (that matches the `wal_state_kernel` module).
    #[test]
    fn inv_wal_pair_fns_are_exactly_the_catalog() {
        freeze_pair(&["inv_wal", "inv_wal_as_is"]);
    }

    /// RFC-0166 P1.2: Inv-WAL is `acked ⊆ synced ⊆ written`; AS-IS drops
    /// the acked⊆synced arm.
    #[test]
    fn theorem_inv_wal_on_finite_domain() {
        let cases: &[(u64, u64, u64)] = &[
            (0, 0, 0),
            (1, 1, 1),
            (2, 3, 10),
            (7, 3, 10),
            (0, 5, 3),
            (3, 3, 10),
            (0, 0, 10),
        ];
        let mut n = 0u32;
        for &(acked, synced, written) in cases {
            let s = WalState {
                acked,
                synced,
                written,
            };
            let fixed = inv_wal(&s);
            let as_is = inv_wal_as_is(&s);
            if acked <= synced && synced <= written {
                assert!(fixed, "FIXED holds {acked}/{synced}/{written}");
            } else {
                assert!(!fixed, "FIXED rejects {acked}/{synced}/{written}");
            }
            if synced <= written {
                assert!(as_is, "AS-IS synced⊆written {acked}/{synced}/{written}");
            } else {
                assert!(!as_is, "AS-IS rejects ceiling {acked}/{synced}/{written}");
            }
            if acked > synced && synced <= written {
                assert!(!fixed && as_is, "AS-IS dente: acked without barrier");
            }
            n += 1;
        }
        assert_eq!(n, 7);
    }

    /// Catalog pair `wal_append`.
    #[test]
    fn wal_append_pair_fns_are_exactly_the_catalog() {
        freeze_pair(&["wal_append", "wal_append_as_is"]);
    }

    /// RFC-0166 P1.2: append grows `written` only; AS-IS acks the same bytes.
    #[test]
    fn theorem_wal_append_on_finite_domain() {
        let s = WalState {
            acked: 0,
            synced: 0,
            written: 0,
        };
        let mut n = 0u32;
        for add in 0u64..7 {
            let real = wal_append(s, add);
            assert_eq!(real.written, s.written + add);
            assert_eq!(real.synced, s.synced);
            assert_eq!(real.acked, s.acked);
            assert!(inv_wal(&real));
            let as_is = wal_append_as_is(s, add);
            assert_eq!(
                as_is.acked,
                s.acked + add,
                "AS-IS acks with the write n={add}"
            );
            assert_eq!(as_is.written, s.written + add);
            if add > 0 {
                assert!(!inv_wal(&as_is));
                assert!(inv_wal_as_is(&as_is));
            }
            n += 1;
        }
        assert_eq!(n, 7);
    }

    /// Catalog pair `wal_sync`. Names `wal_sync_honesty_*` so a cargo
    /// filter does not also run `wal_sync_required` in write-admission.
    #[test]
    fn wal_sync_honesty_pair_fns_are_exactly_the_catalog() {
        freeze_pair(&["wal_sync", "wal_sync_as_is"]);
    }

    /// RFC-0166 P1.2: honest sync promotes pending; lying is a no-op;
    /// AS-IS always pretends it promoted.
    #[test]
    fn theorem_wal_sync_honesty_on_finite_domain() {
        let s = WalState {
            acked: 0,
            synced: 0,
            written: 8,
        };
        let honest = wal_sync(s, SyncHonesty::Honest);
        assert_eq!(honest.synced, s.written);
        assert_eq!(honest.acked, s.acked);
        assert!(inv_wal(&honest));
        let lying = wal_sync(s, SyncHonesty::Lying);
        assert_eq!(lying.synced, s.synced);
        let fake = wal_sync_as_is(s, SyncHonesty::Lying);
        assert_eq!(
            fake.synced, s.written,
            "AS-IS dente: lying Ok pretended to promote"
        );
        assert_ne!(lying, fake);
        let cases: &[SyncHonesty] = &[
            SyncHonesty::Honest,
            SyncHonesty::Lying,
            SyncHonesty::Honest,
            SyncHonesty::Lying,
            SyncHonesty::Honest,
            SyncHonesty::Lying,
            SyncHonesty::Honest,
        ];
        let mut n = 0u32;
        for &h in cases {
            let real = wal_sync(s, h);
            let as_is = wal_sync_as_is(s, h);
            match h {
                SyncHonesty::Honest => assert_eq!(real.synced, s.written),
                SyncHonesty::Lying => assert_eq!(real.synced, s.synced),
            }
            assert_eq!(as_is.synced, s.written);
            n += 1;
        }
        assert_eq!(n, 7);
    }

    /// Catalog pair `wal_ack`.
    #[test]
    fn wal_ack_pair_fns_are_exactly_the_catalog() {
        freeze_pair(&["wal_ack", "wal_ack_as_is"]);
    }

    /// RFC-0166 P1.2: ack only within the barrier; AS-IS acks past it.
    #[test]
    fn theorem_wal_ack_on_finite_domain() {
        let s = wal_state_of(10, 3, 2);
        let mut n = 0u32;
        for k in 0u64..7 {
            let real = wal_ack(s, k);
            if s.acked.saturating_add(k) <= s.synced {
                assert_eq!(real.acked, s.acked + k, "FIXED within-barrier k={k}");
            } else {
                assert_eq!(real, s, "FIXED refuse k={k}");
            }
            assert!(inv_wal(&real));
            let as_is = wal_ack_as_is(s, k);
            assert_eq!(as_is.acked, s.acked + k, "AS-IS unconditional k={k}");
            if s.acked + k > s.synced {
                assert!(!inv_wal(&as_is));
                assert!(inv_wal_as_is(&as_is));
            }
            n += 1;
        }
        assert_eq!(n, 7);
    }

    /// Catalog pair `wal_rotate`. Names `wal_rotate_pair_*` so a cargo
    /// filter does not also run `wal_rotate_decision_on_live_pin_is_not_ok`.
    #[test]
    fn wal_rotate_pair_fns_are_exactly_the_catalog() {
        freeze_pair(&["wal_rotate", "wal_rotate_as_is"]);
    }

    /// RFC-0166 P1.2: rotate only when fully durable; AS-IS always drops.
    #[test]
    fn theorem_wal_rotate_pair_on_finite_domain() {
        let cases: &[WalState] = &[
            WalState {
                acked: 0,
                synced: 0,
                written: 0,
            },
            WalState {
                acked: 5,
                synced: 5,
                written: 5,
            },
            WalState {
                acked: 3,
                synced: 3,
                written: 10,
            },
            WalState {
                acked: 0,
                synced: 0,
                written: 4,
            },
            WalState {
                acked: 1,
                synced: 2,
                written: 2,
            },
            WalState {
                acked: 4,
                synced: 4,
                written: 4,
            },
            WalState {
                acked: 2,
                synced: 3,
                written: 10,
            },
        ];
        let mut n = 0u32;
        for &s in cases {
            let real = wal_rotate(s);
            if s.acked == s.synced && s.synced == s.written {
                assert_eq!(real, wal_state_of(0, 0, 0), "FIXED drop {s:?}");
            } else {
                assert_eq!(real, s, "FIXED keep {s:?}");
            }
            assert!(inv_wal(&real));
            assert_eq!(
                wal_rotate_as_is(s),
                wal_state_of(0, 0, 0),
                "AS-IS always drops {s:?}"
            );
            n += 1;
        }
        assert_eq!(n, 7);
    }

    /// Catalog pair `wal_acked_survives`. Names avoid the `wal_ack` prefix
    /// so a cargo filter for `wal_ack` stays 2/0.
    #[test]
    fn survives_acked_pair_fns_are_exactly_the_catalog() {
        freeze_pair(&["acked_survives_every_legal_crash", "acked_survives_as_is"]);
    }

    /// RFC-0166 P1.2: every legal crash cut keeps the acked prefix; AS-IS
    /// floor-less legality diverges below the barrier.
    #[test]
    fn theorem_survives_acked_on_finite_domain() {
        let s = wal_state_of(10, 3, 3);
        let mut n = 0u32;
        for cut in 0u64..7 {
            assert!(acked_survives_every_legal_crash(&s, cut), "FIXED cut={cut}");
            if cut < s.acked && cut <= s.written {
                assert!(
                    !acked_survives_as_is(&s, cut),
                    "AS-IS dente: below-floor cut={cut}"
                );
            }
            n += 1;
        }
        assert_eq!(n, 7);
    }
}
