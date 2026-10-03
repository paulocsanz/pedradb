//! F-CAMP-2 autopsy: decode every WAL file in a crash image with the
//! engine's own reader and dump (file, logical record count, first/last
//! user key) — plus a targeted search for one key.

#![forbid(unsafe_code)]

use std::io::BufReader;

use pedradb_core::batch::WriteRecord;
use pedradb_core::wal::WalReader;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: wal_dump <dir> [key]");
    let want = std::env::args().nth(2);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n == "CURRENT.log" || n.starts_with("WAL.arch"))
        .collect();
    names.sort();
    for name in &names {
        let path = std::path::Path::new(&dir).join(name);
        let f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                println!("{name}: OPEN ERR {e}");
                continue;
            }
        };
        let mut rd = WalReader::new(BufReader::new(f));
        match rd.collect_all() {
            Ok(records) => {
                let mut hit = String::new();
                if let Some(w) = &want {
                    let n = records.iter().filter(|r| {
                        r.windows(w.len()).any(|win| win == w.as_bytes())
                    }).count();
                    hit = format!(" · key-hits={n}");
                }
                let first_key = records.first().map(|r| ascii_key(r));
                let last_key = records.last().map(|r| ascii_key(r));
                if let Some(w) = &want {
                    for (ri, rec) in records.iter().enumerate() {
                        if let Some(at) = rec.windows(w.len()).position(|win| win == w.as_bytes()) {
                            let lo = at.saturating_sub(24);
                            let hi = (at + w.len() + 16).min(rec.len());
                            let hexs: Vec<String> =
                                rec[lo..hi].iter().map(|b| format!("{b:02x}")).collect();
                            println!(
                                "  HIT rec#{ri}/{n} rec_bytes={} key-at={at} ctx[{lo}..{hi}]={}",
                                rec.len(),
                                hexs.join(""),
                                n = records.len()
                            );
                        }
                    }
                }
                if std::env::args().any(|a| a == "--decode") {
                    for (ri, rec) in records.iter().enumerate() {
                        match WriteRecord::decode(rec) {
                            Ok(wr) => {
                                for op in &wr.ops {
                                    println!(
                                        "  DEC#{ri:02} kind={:?} seq={} key={} vlen={}",
                                        op.kind,
                                        op.sequence,
                                        String::from_utf8_lossy(&op.key),
                                        op.value.len()
                                    );
                                }
                            }
                            Err(e) => println!("  DEC#{ri:02} ERR {e}"),
                        }
                    }
                }
                if std::env::args().any(|a| a == "--full") {
                    for (ri, rec) in records.iter().enumerate() {
                        // payload layout (empirical): seq u64 LE @0, klen u32 LE @8,
                        // key bytes, vlen u32 LE, value bytes
                        if rec.len() >= 12 {
                            let seq = u64::from_le_bytes(rec[0..8].try_into().unwrap());
                            let klen = u32::from_le_bytes(rec[8..12].try_into().unwrap()) as usize;
                            let key = if klen <= rec.len().saturating_sub(12) {
                                String::from_utf8_lossy(&rec[12..12 + klen]).to_string()
                            } else {
                                format!("<klen {klen} overflow>")
                            };
                            let voff = 12 + klen;
                            let vlen = if rec.len() >= voff + 4 {
                                u32::from_le_bytes(rec[voff..voff + 4].try_into().unwrap())
                            } else {
                                u32::MAX
                            };
                            println!("  REC#{ri:02} seq={seq} klen={klen} key={key} vlen={vlen} bytes={}", rec.len());
                        }
                    }
                }
                println!(
                    "{name}: records={} bytes={}{} first={first_key:?} last={last_key:?}",
                    records.len(),
                    std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                    hit
                );
            }
            Err(e) => println!("{name}: DECODE ERR {e}"),
        }
    }
}

/// Best-effort ASCII key extraction from a record payload (keys appear
/// verbatim inside the encoded WriteOp after the seq varint).
fn ascii_key(record: &[u8]) -> String {
    let mut out = String::new();
    for b in record {
        if b.is_ascii_graphic() || *b == b' ' {
            out.push(*b as char);
        } else if !out.is_empty() {
            break;
        }
    }
    out.chars().take(24).collect()
}
