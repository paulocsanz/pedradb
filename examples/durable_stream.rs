//! A durable append stream: publish, peek, ack — on the same kernel.
//!
//! Message + cursor live in one Pedra directory. `peek` does not advance the
//! consumer; `ack` does, in order, after you have applied the payload. Crash
//! between peek and ack redelivers (at-least-once). `next` acks immediately
//! (at-most-once convenience).
//!
//! ```sh
//! cargo run -p pedradb-examples --example durable_stream
//! ```

use pedradb_stream::Stream;

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run() -> pedradb_stream::Result<()> {
    let dir = scratch("stream");
    {
        let mut events = Stream::open(&dir, "events")?;
        assert_eq!(events.publish(b"hello")?, 1);
        assert_eq!(events.publish(b"world")?, 2);

        let first = events.peek("worker")?.expect("msg 1");
        assert_eq!(first.seq, 1);
        assert_eq!(first.data, b"hello");
        // Crash here and reopen: peek would return hello again.
        events.ack("worker", first.seq)?;

        let second = events.next("worker")?.expect("msg 2");
        assert_eq!(second.data, b"world");
        assert!(events.peek("worker")?.is_none());
        events.close()?;
    }

    let events = Stream::open(&dir, "events")?;
    assert_eq!(events.last_seq(), 2);
    assert_eq!(events.consumer_seq("worker"), 2);
    println!("durable_stream: published 2, worker cursor=2 after peek+ack then next");
    events.close()?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn main() -> pedradb_stream::Result<()> {
    run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::run().expect("durable_stream");
    }
}
