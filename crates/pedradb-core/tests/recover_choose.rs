//! EXPLODE sweep: named WAL mutations through production [`WalReader::collect_all`].
//!
//! Each [`RecoverChoice`] is a byte-level `choose`; the reader +
//! [`recover_collect_act`] decide. AS-IS kernel mutants are the teeth on
//! the policy; this file checks the **real** bytes → recover path.

use std::io::Cursor;

use pedradb_core::error::CoreError;
use pedradb_core::wal::recover_choose::{
    apply_recover_choice, choose_expect, explode_choices, ChooseExpect, RecoverChoice,
};
use pedradb_core::wal::writer::WalWriter;
use pedradb_core::wal::WalReader;

fn write_records(recs: &[&[u8]]) -> Vec<u8> {
    let mut w = WalWriter::new(Cursor::new(Vec::new())).unwrap();
    for r in recs {
        w.add_record(r).unwrap();
    }
    w.into_inner().into_inner()
}

/// Collect through a reader instance kept alive so the sweep can also
/// assert the F171 resync re-anchor report (`resync_origin`).
fn collect_with_reader(
    buf: Vec<u8>,
) -> (Result<Vec<Vec<u8>>, CoreError>, WalReader<Cursor<Vec<u8>>>) {
    let mut r = WalReader::new(Cursor::new(buf));
    let (out, err) = r.collect_prefix_all();
    let res = match err {
        Some(e) => Err(e),
        None => Ok(out),
    };
    (res, r)
}

#[test]
fn explode_sweep_three_records() {
    let recs: [&[u8]; 3] = [b"first", b"second", b"third-overwrite"];
    let clean = write_records(&recs);

    for choice in explode_choices(recs.len()) {
        let mut buf = clean.clone();
        assert!(
            apply_recover_choice(&mut buf, choice),
            "choice {choice:?} must apply to a 3-record WAL"
        );
        // Reader instance kept alive so the F171 re-anchor report can be
        // asserted alongside the record outcome.
        let (got, reader) = collect_with_reader(buf);
        match (choose_expect(choice), got) {
            (ChooseExpect::AllRecords, Ok(out)) => {
                assert_eq!(
                    out,
                    vec![
                        b"first".to_vec(),
                        b"second".to_vec(),
                        b"third-overwrite".to_vec()
                    ],
                    "clean image (incl. pure zero tail = padding, not corruption)"
                );
                // F171: nothing was skipped — no re-anchor origin.
                assert!(
                    reader.resync_origin().is_none(),
                    "{choice:?} must not report a resync origin"
                );
            }
            (ChooseExpect::FailStop, Err(e)) => match choice {
                RecoverChoice::FlipCrc { .. } => {
                    assert!(matches!(e, CoreError::Crc { .. }), "CRC fail-stop, got {e}");
                }
                RecoverChoice::ForgeOrphanMiddle { .. } => {
                    let msg = e.to_string();
                    assert!(
                        msg.contains("orphan") || msg.contains("crc"),
                        "orphan/crc fail-stop, got {msg}"
                    );
                }
                RecoverChoice::ForgeZeroHeaderAlive { .. } => {
                    assert!(
                        matches!(e, CoreError::WalZeroHeader { .. }),
                        "F170: live zero header must fail-stop typed, got {e}"
                    );
                }
                other => panic!("unexpected fail-stop choice {other:?}: {e}"),
            },
            (ChooseExpect::PrefixOnly, Ok(out)) => {
                assert!(!out.is_empty(), "torn tail must keep prefix, got {out:?}");
                assert!(
                    out.iter().all(|r| recs.contains(&r.as_slice())),
                    "torn tail invented a record: {out:?}"
                );
                assert!(
                    out.last().map(Vec::as_slice) != Some(recs[2]),
                    "torn tail must not keep the incomplete last record"
                );
            }
            (ChooseExpect::Resync, Ok(out)) => {
                assert!(
                    !out.is_empty(),
                    "resync must not look like an empty WAL ({choice:?})"
                );
                match choice {
                    RecoverChoice::FlipLength { index: 0 }
                    | RecoverChoice::ForgeUnknownType { index: 0 } => {
                        assert!(
                            out.iter()
                                .any(|r| r.as_slice() == recs[1] || r.as_slice() == recs[2]),
                            "first-record resync must still see a later durable record, got {out:?}"
                        );
                    }
                    _ => assert_eq!(
                        out[0], recs[0],
                        "mid-WAL resync must keep the durable prefix ({choice:?})"
                    ),
                }
                // F171 re-anchor report: a damaged record followed by more
                // records re-anchors the walk (origin set); damage on the
                // LAST record walks to EOF — that is a torn tail, origin
                // must stay clean so fail-closed callers don't refuse it.
                let (.., is_last) = match choice {
                    RecoverChoice::FlipLength { index }
                    | RecoverChoice::ForgeUnknownType { index } => (index, index + 1 == recs.len()),
                    other => panic!("unexpected resync choice {other:?}"),
                };
                if is_last {
                    assert!(
                        reader.resync_origin().is_none(),
                        "{choice:?}: torn-tail walk must not report a re-anchor origin"
                    );
                } else {
                    assert!(
                        reader.resync_origin().is_some(),
                        "{choice:?}: walk re-anchored on a later record — origin must be reported"
                    );
                }
            }
            (ChooseExpect::Resync, Err(e)) => {
                // First-record length/type bitrot may exhaust resync with no
                // prefix — fail-stop, never Ok([]).
                assert!(
                    matches!(choice, RecoverChoice::FlipLength { index: 0 }),
                    "resync of mid-WAL {choice:?} must not fail-stop: {e}"
                );
            }
            (expect, other) => panic!("choice {choice:?}: expected {expect:?}, got {other:?}"),
        }
    }
}

#[test]
fn as_is_policy_still_has_teeth() {
    use pedradb_core::wal::recover_kernel::{
        recover_collect_act, recover_collect_act_as_is, RecoverAct, RecoverKind,
    };
    assert_eq!(
        recover_collect_act(RecoverKind::Crc, 1, true, 1, false),
        RecoverAct::FailStop
    );
    assert_eq!(
        recover_collect_act_as_is(RecoverKind::Crc, 1, true, 1),
        RecoverAct::Resync
    );
    assert_eq!(
        recover_collect_act(RecoverKind::Truncated, 0, false, 0, false),
        RecoverAct::FailStop
    );
    assert_eq!(
        recover_collect_act_as_is(RecoverKind::Truncated, 0, false, 0),
        RecoverAct::Stop
    );
    // F170 teeth: the live zero header fail-stops typed; the AS-IS mutant
    // swallowed the rest of the block as padding (Stop = silent prefix loss).
    assert_eq!(
        recover_collect_act(RecoverKind::ZeroHeaderTail, 1, false, 0, false),
        RecoverAct::FailStop
    );
    assert_eq!(
        recover_collect_act_as_is(RecoverKind::ZeroHeaderTail, 1, false, 0),
        RecoverAct::Stop
    );
}
