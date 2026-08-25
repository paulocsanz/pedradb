//! RFC-0050 P0.5: MTCP TLS 1.3 + mTLS lab (own process so OnceLock is isolated).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.push("montanha-tcp");
    if p.exists() {
        return p;
    }
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_montanha-tcp") {
        return PathBuf::from(p);
    }
    panic!("montanha-tcp binary not found at {}", p.display());
}

struct Node {
    child: Child,
    addr: SocketAddr,
    id: u64,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn write_mtls_pems(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let mut ca_params = rcgen::CertificateParams::new(vec!["ca.pedra.test".into()]);
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = rcgen::Certificate::from_params(ca_params).unwrap();
    let params = rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]);
    let cert = rcgen::Certificate::from_params(params).unwrap();
    let cert_pem = cert.serialize_pem_with_signer(&ca).unwrap();
    let key_pem = cert.serialize_private_key_pem();
    let ca_pem = ca.serialize_pem().unwrap();
    let cert_path = dir.join("node.pem");
    let key_path = dir.join("node-key.pem");
    let ca_path = dir.join("ca.pem");
    std::fs::write(&cert_path, cert_pem).unwrap();
    std::fs::write(&key_path, key_pem).unwrap();
    std::fs::write(&ca_path, ca_pem).unwrap();
    (cert_path, key_path, ca_path)
}

fn tls_flags(cert: &Path, key: &Path, ca: &Path) -> Vec<String> {
    vec![
        "--tls-cert".into(),
        cert.display().to_string(),
        "--tls-key".into(),
        key.display().to_string(),
        "--tls-ca".into(),
        ca.display().to_string(),
        "--tls-server-name".into(),
        "localhost".into(),
        "--require-tls".into(),
    ]
}

#[test]
fn require_tls_refuses_cleartext() {
    let out = Command::new(bin())
        .args([
            "node",
            "--require-tls",
            "--id",
            "1",
            "--bind",
            "127.0.0.1:0",
        ])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--require-tls") || err.contains("tls"),
        "stderr={err}"
    );
}

#[test]
fn tcp_tls_mtls_roundtrip() {
    let tmp = {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "pedradb-mtcp-tls-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    };
    let (cert, key, ca) = write_mtls_pems(&tmp);
    pedradb_store::install_from_pem_files(&cert, &key, &ca, "localhost").unwrap();
    let flags = tls_flags(&cert, &key, &ca);

    let ports: Vec<u16> = (0..3).map(|_| free_port()).collect();
    let peers: Vec<(u64, SocketAddr)> = ports
        .iter()
        .enumerate()
        .map(|(i, &p)| ((i as u64) + 1, format!("127.0.0.1:{p}").parse().unwrap()))
        .collect();
    let peer_flags: Vec<String> = peers
        .iter()
        .flat_map(|(id, a)| vec!["--peer".into(), format!("{id}={a}")])
        .collect();

    let mut nodes = Vec::new();
    for (id, addr) in &peers {
        let data = tmp.join(format!("n{id}"));
        std::fs::create_dir_all(&data).unwrap();
        let mut cmd = Command::new(bin());
        cmd.arg("node")
            .arg("--id")
            .arg(id.to_string())
            .arg("--data")
            .arg(&data)
            .arg("--bind")
            .arg(addr.to_string())
            .arg("--ranges")
            .arg("1")
            .args(&peer_flags)
            .args(&flags)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn().expect("spawn node");
        nodes.push(Node {
            child,
            addr: *addr,
            id: *id,
        });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    for n in &nodes {
        loop {
            if pedradb_store::client_status(n.addr.to_string()).is_ok() {
                break;
            }
            if Instant::now() > deadline {
                panic!("tls node {} not up at {}", n.id, n.addr);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    let mut elect = Command::new(bin());
    elect.arg("elect-wait").args(&peer_flags).args(&flags);
    let status = elect.output().expect("elect-wait");
    assert!(
        status.status.success(),
        "elect-wait failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let addr = nodes[0].addr.to_string();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut put_ok = false;
    while Instant::now() < deadline {
        if pedradb_store::client_put(&addr, b"tls-k", b"tls-v").is_ok() {
            put_ok = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(put_ok, "tls put");
    let got = pedradb_store::client_get(&addr, b"tls-k").expect("tls get");
    assert_eq!(got.as_deref(), Some(&b"tls-v"[..]));

    // RFC-0050 P1.3: health HTTP is TLS when PEMs are installed.
    let hp = nodes[0].addr.port().saturating_add(79);
    let ha: SocketAddr = format!("127.0.0.1:{hp}").parse().unwrap();
    let hdead = Instant::now() + Duration::from_secs(8);
    let mut health_ok = false;
    while Instant::now() < hdead {
        if let Ok(s) = TcpStream::connect_timeout(&ha, Duration::from_millis(200)) {
            if let Ok(mut s) = pedradb_store::maybe_client_wrap(s) {
                let _ = s.write_all(b"GET /ready HTTP/1.0\r\nHost: localhost\r\n\r\n");
                let mut buf = [0u8; 256];
                if let Ok(n) = s.read(&mut buf) {
                    let body = String::from_utf8_lossy(&buf[..n]);
                    if body.contains("200") || body.contains("ready") {
                        health_ok = true;
                        break;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(health_ok, "health HTTPS /ready on {ha}");
}

#[test]
fn tls_reload_from_pem_files_replaces_config() {
    let tmp = {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "pedradb-tls-reload-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    };
    let (cert, key, ca) = write_mtls_pems(&tmp);
    pedradb_store::install_from_pem_files(&cert, &key, &ca, "localhost").unwrap();
    // Second call used to fail "already installed" (OnceLock). Rotation must replace.
    pedradb_store::reload_from_pem_files(&cert, &key, &ca, "localhost").unwrap();
    pedradb_store::install_from_pem_files(&cert, &key, &ca, "localhost").unwrap();
    assert!(pedradb_store::tls_installed());
    let _ = std::fs::remove_dir_all(&tmp);
}
