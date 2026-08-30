//! Corruption journal + escalation policy (RFC-0038 option **D**).
//!
//! A single mid-WAL CRC event cannot distinguish an isolated bitflip (disk is
//! fine: quarantine + report is the right answer) from progressive media
//! failure (disk is dying: evacuate). Only **history** discriminates. This
//! module appends one line per fail-stop corruption observed at open and
//! refuses the Nth such event — in every mode — with
//! [`CoreError::CorruptionEscalated`], forcing evacuation instead of
//! silent continued service on dying hardware.
//!
//! Semantics (deliberate):
//! - only **fail-stop** recoveries journal (CRC; bitrot-style `Truncated(0)`
//!   head). Resyncable torn tails never journal — they are routine.
//! - the journal never blocks a **clean** open: after the operator repairs or
//!   replaces the WAL (or evacuates), open proceeds normally.
//! - journal write failures never mask the original corruption error.

use std::io::{Read, Write};
use std::path::Path;

use crate::env::{Env, EnvFile};
use crate::error::CoreError;

/// Append-only corruption journal, one tab-separated line per event.
pub const CORRUPTLOG_NAME: &str = "CORRUPTLOG";

/// Repeated fail-stop events at/after this count escalate (RFC-0038 P2.1).
pub const CORRUPTION_ESCALATION_EVENTS: u32 = 3;

/// Append one event line and return the total number of events recorded.
fn record_event<E: Env>(env: &E, dir: &Path, kind: &str, offset: u64) -> std::io::Result<u32> {
    let path = dir.join(CORRUPTLOG_NAME);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut f = env.open_append(&path).or_else(|_| env.create(&path))?;
    writeln!(f, "{ts}\t{kind}\t{offset}")?;
    f.flush()?;
    f.sync_data()?;
    count_events(env, dir)
}

/// Number of events in the journal (0 when absent/unreadable-but-existing).
fn count_events<E: Env>(env: &E, dir: &Path) -> std::io::Result<u32> {
    let path = dir.join(CORRUPTLOG_NAME);
    if !env.exists(&path) {
        return Ok(0);
    }
    // Read-capable handle: `open_append` is write-only (append mode) and
    // read(2) on it fails with EBADF.
    let mut f = env.open_read(&path)?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)?;
    Ok(buf.lines().filter(|l| !l.trim().is_empty()).count() as u32)
}

/// Journal a fail-stop corruption; on the Nth recorded event, escalate.
///
/// Journal I/O failures never mask `original`: worst case the event is not
/// counted and the caller still sees the corruption error.
pub(crate) fn escalate_or_fail<E: Env>(
    env: &E,
    dir: &Path,
    kind: &str,
    offset: u64,
    original: CoreError,
) -> CoreError {
    match record_event(env, dir, kind, offset) {
        Ok(events) if events >= CORRUPTION_ESCALATION_EVENTS => CoreError::CorruptionEscalated {
            events,
            limit: CORRUPTION_ESCALATION_EVENTS,
        },
        _ => original,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StdEnv;

    fn temp_dir() -> std::path::PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedradb-corrupt-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn journal_counts_events_and_escalates_at_limit() {
        let dir = temp_dir();
        let env = StdEnv;
        for expected in 1..CORRUPTION_ESCALATION_EVENTS {
            let e = escalate_or_fail(&env, &dir, "crc", 42, CoreError::Truncated(0));
            assert!(
                matches!(e, CoreError::Truncated(0)),
                "event {expected} must not escalate"
            );
            assert_eq!(count_events(&env, &dir).unwrap(), expected);
        }
        let e = escalate_or_fail(&env, &dir, "crc", 42, CoreError::Truncated(0));
        match e {
            CoreError::CorruptionEscalated { events, limit } => {
                assert_eq!(events, CORRUPTION_ESCALATION_EVENTS);
                assert_eq!(limit, CORRUPTION_ESCALATION_EVENTS);
            }
            other => panic!("expected escalation, got {other:?}"),
        }
        let raw = std::fs::read_to_string(dir.join(CORRUPTLOG_NAME)).unwrap();
        assert_eq!(raw.lines().count() as u32, CORRUPTION_ESCALATION_EVENTS);
        assert!(raw.lines().all(|l| l.contains("\tcrc\t")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
