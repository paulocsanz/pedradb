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

fn collect(buf: Vec<u8>) -> Result<Vec<Vec<u8>>, CoreError> {
    WalReader::new(Cursor::new(buf)).collect_all()
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
        let got = collect(buf);
        match (choose_expect(choice), got) {
            (ChooseExpect::AllRecords, Ok(out)) => {
                assert_eq!(
                    out,
                    vec![
                        b"first".to_vec(),
                        b"second".to_vec(),
                        b"third-overwrite".to_vec()
                    ],
                    "clean image"
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
        recover_collect_act(RecoverKind::Crc, 1, true, 1),
        RecoverAct::FailStop
    );
    assert_eq!(
        recover_collect_act_as_is(RecoverKind::Crc, 1, true, 1),
        RecoverAct::Resync
    );
    assert_eq!(
        recover_collect_act(RecoverKind::Truncated, 0, false, 0),
        RecoverAct::FailStop
    );
    assert_eq!(
        recover_collect_act_as_is(RecoverKind::Truncated, 0, false, 0),
        RecoverAct::Stop
    );
}
