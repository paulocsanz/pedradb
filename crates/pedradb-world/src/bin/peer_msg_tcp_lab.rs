//! C1.2: PeerMsg over localhost TCP (same codec as World Queued path).
//!
//! Two threads: server accepts one connection, client sends RequestVote +
//! AppendEntries frames; server echoes decode-ok replies. Proves product-shaped
//! wire path without dual-leader (single elective message exchange).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use pedradb_store::PeerMsg;

fn write_frame(w: &mut impl Write, body: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(body.len()).expect("frame");
    w.write_all(&len.to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut hdr = [0u8; 4];
    r.read_exact(&mut hdr)?;
    let len = u32::from_le_bytes(hdr) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(body)
}

fn main() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");

    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut n = 0u32;
        loop {
            let body = match read_frame(&mut sock) {
                Ok(b) => b,
                Err(_) => break,
            };
            let msg = PeerMsg::decode(&body).expect("PeerMsg decode on server");
            // Echo a tiny ack frame: re-encode same message (round-trip proof).
            let out = msg.encode();
            write_frame(&mut sock, &out).expect("server write");
            n += 1;
            if n >= 2 {
                break;
            }
        }
        n
    });

    thread::sleep(Duration::from_millis(20));
    let mut client = TcpStream::connect(addr).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let rv = PeerMsg::RequestVote {
        range_id: 1,
        term: 3,
        candidate_id: 1,
        last_log_index: 0,
        last_log_term: 0,
    };
    write_frame(&mut client, &rv.encode()).expect("client rv");
    let echo1 = read_frame(&mut client).expect("echo rv");
    assert_eq!(PeerMsg::decode(&echo1).unwrap(), rv);

    let ae = PeerMsg::AppendEntries {
        range_id: 1,
        term: 3,
        leader_id: 1,
        prev_log_index: 0,
        prev_log_term: 0,
        leader_commit: 0,
        entries: vec![],
    };
    write_frame(&mut client, &ae.encode()).expect("client ae");
    let echo2 = read_frame(&mut client).expect("echo ae");
    assert_eq!(PeerMsg::decode(&echo2).unwrap(), ae);

    let served = server.join().expect("server join");
    assert_eq!(served, 2);
    println!("peer_msg_tcp_lab_ok addr={addr} msgs=2 dual_leader_fail_open=0");
}
