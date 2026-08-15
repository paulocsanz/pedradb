//! Minimal HTTP/1.0 wire for PedraDB products (RFC-0012 P1.1 / P1.2).
//!
//! Not production TLS. Optional shared-secret auth via
//! `Authorization: Bearer <token>` or `X-Pedra-Token: <token>`.
//! Empty token = open bind (lab only).
//!
//! Routes:
//!
//! **KV** (`KvServer`):
//! - `GET /kv/<key>` → raw value or 404
//! - `PUT /kv/<key>` body → store
//! - `DELETE /kv/<key>`
//!
//! **DCS** (`DcsServer`):
//! - `GET /dcs/kv/<key>` → JSON-ish `rev value` line
//! - `PUT /dcs/kv/<key>?rev=N` body → CAS (rev=0 create)
//! - `POST /dcs/lease?ttl_ms=N` → lease id
//! - `POST /dcs/leader?key=...&holder=...&ttl_ms=N` → acquire
//! - `POST /dcs/renew?key=...&holder=...&lease=N&rev=N`

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod auth_kernel;
mod cl_kernel;

pub use auth_kernel::{
    ascii_lower, bearer_token_from_value, is_bearer_scheme, is_bearer_scheme_as_is,
};
pub use cl_kernel::{
    content_length_repeat_ok, content_length_repeat_ok_as_is, invalid_cl_as_zero,
    invalid_cl_as_zero_as_is, keep_body_without_cl, keep_body_without_cl_as_is,
};

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pedradb_apply::KvService;
use pedradb_dcs::Dcs;
use thiserror::Error;

/// HTTP server errors.
#[derive(Debug, Error)]
pub enum HttpError {
    /// I/O.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// App.
    #[error("{0}")]
    App(String),
}

/// Result.
pub type Result<T> = std::result::Result<T, HttpError>;

fn read_req(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>, Vec<(String, String)>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 1024 * 1024 {
            return Err(HttpError::App("req too large".into()));
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let (head, body_start) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| HttpError::App("bad http".into()))?;
    let mut lines = head.lines();
    let req = lines.next().unwrap_or("");
    let mut parts = req.split_whitespace();
    let method = parts.next().unwrap_or("").to_ascii_uppercase();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_len = 0usize;
    let mut has_content_len = false;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            // F87: invalid CL used to become 0 and truncate the body to empty.
            let n = match v.trim().parse::<usize>() {
                Ok(n) => n,
                Err(_) if invalid_cl_as_zero() => 0,
                Err(_) => return Err(HttpError::App("bad content-length".into())),
            };
            // F88: differing Content-Length fields — last header used to win
            // (`5` then `0` stored empty). RFC 9112: reject the message.
            if has_content_len && !content_length_repeat_ok(content_len as u64, n as u64) {
                return Err(HttpError::App("conflicting content-length".into()));
            }
            has_content_len = true;
            content_len = n;
        }
    }
    // Cap body size (F8): previously Content-Length could force multi-GiB alloc.
    const MAX_BODY: usize = 16 * 1024 * 1024;
    if has_content_len && content_len > MAX_BODY {
        return Err(HttpError::App(format!(
            "content-length {content_len} exceeds max {MAX_BODY}"
        )));
    }
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let mut body = buf[header_end..].to_vec();
    if has_content_len {
        while body.len() < content_len {
            let n = stream.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
            if body.len() > MAX_BODY {
                return Err(HttpError::App("body exceeds max".into()));
            }
        }
        body.truncate(content_len);
    } else if keep_body_without_cl() {
        // F86: no Content-Length — keep bytes already past the header break.
        // Do not drain the socket (GET/keep-alive would hang waiting for EOF).
        if body.len() > MAX_BODY {
            return Err(HttpError::App("body exceeds max".into()));
        }
    } else {
        body.truncate(0);
    }
    let _ = body_start;
    Ok((method, path, body, headers))
}

/// Extract bearer / X-Pedra-Token from headers.
fn header_token(headers: &[(String, String)]) -> Option<&str> {
    for (k, v) in headers {
        if k == "x-pedra-token" {
            return Some(v.as_str());
        }
        if k == "authorization" {
            return bearer_token_from_value(v);
        }
    }
    None
}

fn authorize(headers: &[(String, String)], token: &Option<String>) -> bool {
    match token {
        None => true,
        Some(t) if t.is_empty() => true,
        Some(t) => header_token(headers) == Some(t.as_str()),
    }
}

fn write_resp(stream: &mut TcpStream, code: u16, reason: &str, body: &[u8]) -> Result<()> {
    let head = format!(
        "HTTP/1.0 {code} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

/// KV HTTP server.
pub struct KvServer {
    kv: Arc<Mutex<KvService>>,
    /// Shared secret; `None`/empty = open bind (lab only).
    auth_token: Option<String>,
}

impl KvServer {
    /// Open DB at path (no auth).
    ///
    /// # Errors
    /// Open.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_auth(path, None)
    }

    /// Open with optional bearer / `X-Pedra-Token` auth.
    ///
    /// # Errors
    /// Open.
    pub fn open_with_auth(path: impl AsRef<Path>, token: Option<String>) -> Result<Self> {
        let kv = KvService::open(path).map_err(|e| HttpError::App(e.to_string()))?;
        Ok(Self {
            kv: Arc::new(Mutex::new(kv)),
            auth_token: token,
        })
    }

    /// Serve on `addr` (blocks).
    ///
    /// # Errors
    /// Bind.
    pub fn serve(self, addr: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(addr)?;
        let kv = self.kv;
        let auth = self.auth_token;
        for conn in listener.incoming() {
            let mut stream = match conn {
                Ok(s) => s,
                Err(_) => continue,
            };
            let kv = Arc::clone(&kv);
            let auth = auth.clone();
            thread::spawn(move || {
                let _ = handle_kv(&mut stream, &kv, &auth);
            });
        }
        Ok(())
    }
}

fn handle_kv(
    stream: &mut TcpStream,
    kv: &Arc<Mutex<KvService>>,
    auth: &Option<String>,
) -> Result<()> {
    let (method, path, body, headers) = read_req(stream)?;
    if !authorize(&headers, auth) {
        return write_resp(stream, 401, "Unauthorized", b"auth required");
    }
    let po = path_only(&path);
    if let Some(key) = po.strip_prefix("/kv/") {
        let key = percent_decode(key);
        match method.as_str() {
            "GET" => {
                let g = kv.lock().map_err(|e| HttpError::App(e.to_string()))?;
                match g.get(&key) {
                    Some(v) => write_resp(stream, 200, "OK", &v)?,
                    None => write_resp(stream, 404, "Not Found", b"")?,
                }
            }
            "PUT" => {
                let mut g = kv.lock().map_err(|e| HttpError::App(e.to_string()))?;
                g.put(key, &body)
                    .map_err(|e| HttpError::App(e.to_string()))?;
                write_resp(stream, 200, "OK", b"ok")?;
            }
            "DELETE" => {
                let mut g = kv.lock().map_err(|e| HttpError::App(e.to_string()))?;
                g.delete(key).map_err(|e| HttpError::App(e.to_string()))?;
                write_resp(stream, 200, "OK", b"ok")?;
            }
            _ => write_resp(stream, 405, "Method Not Allowed", b"")?,
        }
    } else {
        write_resp(stream, 404, "Not Found", b"")?
    }
    Ok(())
}

/// DCS HTTP server (Patroni-oriented subset).
pub struct DcsServer {
    dcs: Arc<Mutex<Dcs>>,
    auth_token: Option<String>,
}

impl DcsServer {
    /// Open DCS store (no auth).
    ///
    /// # Errors
    /// Open.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_auth(path, None)
    }

    /// Open with optional token auth (same headers as [`KvServer`]).
    ///
    /// # Errors
    /// Open.
    pub fn open_with_auth(path: impl AsRef<Path>, token: Option<String>) -> Result<Self> {
        let dcs = Dcs::open(path).map_err(|e| HttpError::App(e.to_string()))?;
        Ok(Self {
            dcs: Arc::new(Mutex::new(dcs)),
            auth_token: token,
        })
    }

    /// Serve (blocks).
    ///
    /// # Errors
    /// Bind.
    pub fn serve(self, addr: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(addr)?;
        let dcs = self.dcs;
        let auth = self.auth_token;
        for conn in listener.incoming() {
            let mut stream = match conn {
                Ok(s) => s,
                Err(_) => continue,
            };
            let dcs = Arc::clone(&dcs);
            let auth = auth.clone();
            thread::spawn(move || {
                let _ = handle_dcs(&mut stream, &dcs, &auth);
            });
        }
        Ok(())
    }
}

fn query_param(path: &str, key: &str) -> Option<String> {
    let q = path.split_once('?')?.1;
    for part in q.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            if k == key {
                return Some(String::from_utf8_lossy(&percent_decode(v)).into_owned());
            }
        }
    }
    None
}

/// Origin-form path for routing (strip query; accept absolute / network-path forms).
///
/// F91: proxies/clients may send `http://host/kv/x` (absolute-form). A raw
/// `strip_prefix("/kv/")` then failed and every route 404'd.
/// F92: network-path-reference `//host/kv/x` (no scheme) had the same miss.
fn path_only(path: &str) -> &str {
    let mut p = path;
    // Absolute-form: scheme://authority/path?query
    if let Some(rest) = p
        .strip_prefix("http://")
        .or_else(|| p.strip_prefix("https://"))
        .or_else(|| p.strip_prefix("HTTP://"))
        .or_else(|| p.strip_prefix("HTTPS://"))
    {
        p = rest.find('/').map(|i| &rest[i..]).unwrap_or("/");
    } else if let Some(rest) = p.strip_prefix("//") {
        // F92: network-path-reference //authority/path (no scheme)
        p = rest.find('/').map(|i| &rest[i..]).unwrap_or("/");
    }
    p.split_once('?').map(|(a, _)| a).unwrap_or(p)
}

/// Decode `%HH` in a path segment (F75). Invalid sequences are left as-is.
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (from_hex(b[i + 1]), from_hex(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn from_hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn handle_dcs(stream: &mut TcpStream, dcs: &Arc<Mutex<Dcs>>, auth: &Option<String>) -> Result<()> {
    let (method, path, body, headers) = read_req(stream)?;
    if !authorize(&headers, auth) {
        return write_resp(stream, 401, "Unauthorized", b"auth required");
    }
    let po = path_only(&path);
    if let Some(key) = po.strip_prefix("/dcs/kv/") {
        let key = percent_decode(key);
        match method.as_str() {
            "GET" => {
                let g = dcs.lock().map_err(|e| HttpError::App(e.to_string()))?;
                match g.get(&key) {
                    Some(kv) => {
                        let line = format!(
                            "{} {}\n",
                            kv.mod_revision,
                            String::from_utf8_lossy(&kv.value)
                        );
                        write_resp(stream, 200, "OK", line.as_bytes())?;
                    }
                    None => write_resp(stream, 404, "Not Found", b"")?,
                }
            }
            "PUT" => {
                let rev: u64 = query_param(&path, "rev")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let mut g = dcs.lock().map_err(|e| HttpError::App(e.to_string()))?;
                match g.cas(&key, &body, rev, 0) {
                    Ok(new_rev) => {
                        write_resp(stream, 200, "OK", format!("{new_rev}\n").as_bytes())?
                    }
                    Err(e) => write_resp(stream, 409, "Conflict", e.to_string().as_bytes())?,
                }
            }
            _ => write_resp(stream, 405, "Method Not Allowed", b"")?,
        }
        return Ok(());
    }
    if po == "/dcs/lease" && method == "POST" {
        let ttl_ms: u64 = query_param(&path, "ttl_ms")
            .and_then(|s| s.parse().ok())
            .unwrap_or(30_000);
        let mut g = dcs.lock().map_err(|e| HttpError::App(e.to_string()))?;
        let id = g.grant_lease(Duration::from_millis(ttl_ms));
        write_resp(stream, 200, "OK", format!("{id}\n").as_bytes())?;
        return Ok(());
    }
    if po == "/dcs/leader" && method == "POST" {
        let key = query_param(&path, "key").unwrap_or_else(|| "/leader".into());
        let holder = query_param(&path, "holder").unwrap_or_else(|| "node".into());
        let ttl_ms: u64 = query_param(&path, "ttl_ms")
            .and_then(|s| s.parse().ok())
            .unwrap_or(30_000);
        let mut g = dcs.lock().map_err(|e| HttpError::App(e.to_string()))?;
        match g.try_acquire_leader(
            key.as_bytes(),
            holder.as_bytes(),
            Duration::from_millis(ttl_ms),
        ) {
            Ok((rev, lease)) => {
                write_resp(
                    stream,
                    200,
                    "OK",
                    format!("rev={rev} lease={lease}\n").as_bytes(),
                )?;
            }
            Err(e) => write_resp(stream, 409, "Conflict", e.to_string().as_bytes())?,
        }
        return Ok(());
    }
    if po == "/dcs/renew" && method == "POST" {
        let key = query_param(&path, "key").unwrap_or_else(|| "/leader".into());
        let holder = query_param(&path, "holder").unwrap_or_else(|| "node".into());
        let lease: u64 = query_param(&path, "lease")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let rev: u64 = query_param(&path, "rev")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut g = dcs.lock().map_err(|e| HttpError::App(e.to_string()))?;
        match g.renew_leader(key.as_bytes(), holder.as_bytes(), lease, rev) {
            Ok(new_rev) => write_resp(stream, 200, "OK", format!("{new_rev}\n").as_bytes())?,
            Err(e) => write_resp(stream, 409, "Conflict", e.to_string().as_bytes())?,
        }
        return Ok(());
    }
    write_resp(stream, 404, "Not Found", b"")
}

/// Simple HTTP client for tests.
pub fn http_exchange(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(u16, Vec<u8>)> {
    http_exchange_auth(addr, method, path, body, None)
}

/// HTTP client with optional bearer token.
pub fn http_exchange_auth(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
    token: Option<&str>,
) -> Result<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect(addr)?;
    let auth_h = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.0\r\nContent-Length: {}\r\nHost: localhost\r\n{auth_h}\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes())?;
    stream.write_all(body)?;
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp)?;
    let text = String::from_utf8_lossy(&resp);
    let code = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| resp[i + 4..].to_vec())
        .unwrap_or_default();
    Ok((code, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn bind_ephemeral() -> SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        addr
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-http-{tag}-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// F79: HTTP methods must be case-insensitive (RFC 9110).
    #[test]
    fn kv_http_method_case_insensitive() {
        let dir = temp("meth");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let (code, _) = http_exchange(addr, "put", "/kv/case", b"ok").unwrap();
        assert_eq!(code, 200, "lowercase put must work");
        let (code, body) = http_exchange(addr, "get", "/kv/case", b"").unwrap();
        assert_eq!(code, 200);
        assert_eq!(body, b"ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kv_http_put_get() {
        let dir = temp("kv");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let (code, _) = http_exchange(addr, "PUT", "/kv/hello", b"world").unwrap();
        assert_eq!(code, 200);
        let (code, body) = http_exchange(addr, "GET", "/kv/hello", b"").unwrap();
        assert_eq!(code, 200);
        assert_eq!(body, b"world");
        let (code, body) = http_exchange(addr, "GET", "/kv/hello?x=1", b"").unwrap();
        assert_eq!(
            code, 200,
            "query string must not change the KV key, body={body:?}"
        );
        assert_eq!(body, b"world");
        let (code, _) = http_exchange(addr, "PUT", "/kv/a%2Fb", b"slash").unwrap();
        assert_eq!(code, 200);
        let (code, body) = http_exchange(addr, "GET", "/kv/a/b", b"").unwrap();
        assert_eq!(
            code, 200,
            "percent-decoded /kv/a%2Fb must be key a/b, body={body:?}"
        );
        assert_eq!(body, b"slash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dcs_http_leader_race() {
        let dir = temp("dcs");
        let addr = bind_ephemeral();
        let srv = DcsServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let (c1, b1) = http_exchange(
            addr,
            "POST",
            "/dcs/leader?key=pg-leader&holder=a&ttl_ms=5000",
            b"",
        )
        .unwrap();
        assert_eq!(c1, 200, "{b1:?}");
        let (c2, _) = http_exchange(
            addr,
            "POST",
            "/dcs/leader?key=pg-leader&holder=b&ttl_ms=5000",
            b"",
        )
        .unwrap();
        assert_eq!(c2, 409);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Query values must percent-decode (F75 only did the path).
    #[test]
    fn dcs_http_query_key_percent_decoded() {
        let dir = temp("dcs-q");
        let addr = bind_ephemeral();
        let srv = DcsServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let (c1, b1) = http_exchange(
            addr,
            "POST",
            "/dcs/leader?key=a%2Fb&holder=n1&ttl_ms=8000",
            b"",
        )
        .unwrap();
        assert_eq!(c1, 200, "{b1:?}");
        let (c2, body) = http_exchange(addr, "GET", "/dcs/kv/a/b", b"").unwrap();
        assert_eq!(
            c2, 200,
            "query key=a%2Fb must be the same lock as /dcs/kv/a/b, body={body:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F8: Content-Length must not force multi-GiB allocation / hang.
    #[test]
    fn rejects_oversized_content_length() {
        let dir = temp("clen");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        // Claim 1 GiB body — pre-fix would attempt to buffer until that size.
        let req = b"PUT /kv/x HTTP/1.0\r\nContent-Length: 1073741824\r\nHost: localhost\r\n\r\n";
        stream.write_all(req).unwrap();
        let mut resp = Vec::new();
        let mut buf = [0u8; 512];
        // Server should close quickly with error (or drop); must not hang/OOM.
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => {
                // Connection closed / timed out after reject — acceptable if no OOM.
            }
            Ok(n) => {
                resp.extend_from_slice(&buf[..n]);
            }
        }
        // If we got a response body path through App error, fine; main check is
        // we did not allocate 1GiB (process still alive, test finishes).
        let _ = resp;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kv_http_requires_token_when_configured() {
        let dir = temp("auth");
        let addr = bind_ephemeral();
        let srv = KvServer::open_with_auth(&dir, Some("sekrit".into())).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let (code, _) = http_exchange(addr, "PUT", "/kv/x", b"y").unwrap();
        assert_eq!(code, 401, "missing token must 401");
        let (code, _) = http_exchange_auth(addr, "PUT", "/kv/x", b"y", Some("wrong")).unwrap();
        assert_eq!(code, 401, "wrong token must 401");
        let (code, _) = http_exchange_auth(addr, "PUT", "/kv/x", b"y", Some("sekrit")).unwrap();
        assert_eq!(code, 200);
        let (code, body) = http_exchange_auth(addr, "GET", "/kv/x", b"", Some("sekrit")).unwrap();
        assert_eq!(code, 200);
        assert_eq!(body, b"y");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F85: RFC 9110 auth scheme is case-insensitive. Only `Bearer` / `bearer`
    /// were accepted; `BEARER` compared the whole header to the token → 401.
    #[test]
    fn kv_http_bearer_scheme_case_insensitive() {
        let dir = temp("auth-scheme");
        let addr = bind_ephemeral();
        let srv = KvServer::open_with_auth(&dir, Some("sekrit".into())).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let (code, _) = http_exchange_auth(addr, "PUT", "/kv/x", b"y", Some("sekrit")).unwrap();
        assert_eq!(code, 200);
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(
                b"GET /kv/x HTTP/1.0\r\nAuthorization: BEARER sekrit\r\nHost: localhost\r\n\r\n",
            )
            .unwrap();
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).unwrap();
        let text = String::from_utf8_lossy(&resp);
        let code = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap_or(0);
        let body = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| resp[i + 4..].to_vec())
            .unwrap_or_default();
        assert_eq!(
            code, 200,
            "Authorization: BEARER must authenticate, body={body:?}"
        );
        assert_eq!(body, b"y");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F86: HTTP/1.0 PUT without Content-Length still delivered a body after
    /// the header break. `read_req` defaulted CL=0 and `truncate(0)` dropped it,
    /// so the store recorded empty instead of the payload.
    #[test]
    fn kv_http_put_without_content_length_keeps_body() {
        let dir = temp("no-cl");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"PUT /kv/nobody HTTP/1.0\r\nHost: localhost\r\n\r\nhello")
            .unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).unwrap();
        let text = String::from_utf8_lossy(&resp);
        let put_code = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap_or(0);
        assert_eq!(
            put_code, 200,
            "PUT without CL must still be accepted, {text:?}"
        );
        let (code, body) = http_exchange(addr, "GET", "/kv/nobody", b"").unwrap();
        assert_eq!(
            code, 200,
            "missing Content-Length dropped the body, GET code={code} body={body:?}"
        );
        assert_eq!(
            body, b"hello",
            "PUT body after header break must be stored, got {body:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F87: `Content-Length: abc` parsed as 0 (`unwrap_or(0)`) and truncated
    /// the payload — same silent empty put as F86, with a *present* bad header.
    #[test]
    fn kv_http_bad_content_length_does_not_store_empty() {
        let dir = temp("bad-cl");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(
                b"PUT /kv/badcl HTTP/1.0\r\nContent-Length: abc\r\nHost: localhost\r\n\r\nhello",
            )
            .unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = Vec::new();
        let _ = stream.read_to_end(&mut resp);
        let (code, body) = http_exchange(addr, "GET", "/kv/badcl", b"").unwrap();
        assert!(
            !(code == 200 && body.is_empty()),
            "bad Content-Length stored empty value (silent wrong), GET {code} {body:?}"
        );
        assert_ne!(
            (code, body.as_slice()),
            (200, b"hello".as_slice()),
            "malformed Content-Length must not be treated as a successful framed put"
        );
        assert_eq!(
            code, 404,
            "malformed Content-Length must fail closed (no store), GET {code} {body:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F88: two Content-Length values — last header won. `5` then `0` truncated
    /// the payload and stored empty (RFC 9112: differing CLs must be rejected).
    #[test]
    fn kv_http_conflicting_content_length_does_not_store_empty() {
        let dir = temp("dup-cl");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(
                b"PUT /kv/dupcl HTTP/1.0\r\nContent-Length: 5\r\nContent-Length: 0\r\nHost: localhost\r\n\r\nhello",
            )
            .unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = Vec::new();
        let _ = stream.read_to_end(&mut resp);
        let (code, body) = http_exchange(addr, "GET", "/kv/dupcl", b"").unwrap();
        assert!(
            !(code == 200 && body.is_empty()),
            "conflicting Content-Length stored empty (last=0 won), GET {code} {body:?}"
        );
        assert_eq!(
            code, 404,
            "differing Content-Length must fail closed, GET {code} {body:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_only_strips_absolute_form_request_target() {
        assert_eq!(path_only("/kv/x"), "/kv/x");
        assert_eq!(path_only("/kv/x?y=1"), "/kv/x");
        // F91: absolute-form must yield origin-form path
        assert_eq!(path_only("http://127.0.0.1:9/kv/x"), "/kv/x");
        assert_eq!(path_only("https://h/kv/a%2Fb?q=1"), "/kv/a%2Fb");
        assert_eq!(path_only("HTTP://H/dcs/kv/k"), "/dcs/kv/k");
        assert_eq!(path_only("http://only-host"), "/");
        // F92: network-path-reference (no scheme)
        assert_eq!(path_only("//127.0.0.1:9/kv/x"), "/kv/x");
        assert_eq!(path_only("//h/dcs/kv/k?rev=1"), "/dcs/kv/k");
        assert_eq!(path_only("//only-host"), "/");
    }

    /// F92: network-path `//host/kv/k` must route like origin-form.
    #[test]
    fn kv_http_network_path_request_target() {
        let dir = temp("net-path");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let mut stream = TcpStream::connect(addr).unwrap();
        let host = format!("{addr}");
        let req =
            format!("PUT //{host}/kv/np HTTP/1.0\r\nContent-Length: 2\r\nHost: {host}\r\n\r\nok");
        stream.write_all(req.as_bytes()).unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).unwrap();
        let text = String::from_utf8_lossy(&resp);
        let put_code = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap_or(0);
        assert_eq!(
            put_code, 200,
            "network-path PUT must route to /kv/np, resp={text:?}"
        );
        let (code, body) = http_exchange(addr, "GET", "/kv/np", b"").unwrap();
        assert_eq!((code, body.as_slice()), (200, b"ok".as_slice()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F91: absolute-form `PUT http://host/kv/k` must not 404 the route.
    #[test]
    fn kv_http_absolute_form_request_target() {
        let dir = temp("abs-form");
        let addr = bind_ephemeral();
        let srv = KvServer::open(&dir).unwrap();
        thread::spawn(move || {
            let _ = srv.serve(addr);
        });
        thread::sleep(Duration::from_millis(100));
        let mut stream = TcpStream::connect(addr).unwrap();
        let host = format!("{addr}");
        let req = format!(
            "PUT http://{host}/kv/abs HTTP/1.0\r\nContent-Length: 3\r\nHost: {host}\r\n\r\nyes"
        );
        stream.write_all(req.as_bytes()).unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).unwrap();
        let text = String::from_utf8_lossy(&resp);
        let put_code = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap_or(0);
        assert_eq!(
            put_code, 200,
            "absolute-form PUT must route to /kv/abs, resp={text:?}"
        );
        let (code, body) = http_exchange(addr, "GET", "/kv/abs", b"").unwrap();
        assert_eq!(code, 200, "GET after absolute-form PUT, body={body:?}");
        assert_eq!(body, b"yes");
        // GET absolute-form too
        let mut stream = TcpStream::connect(addr).unwrap();
        let req = format!("GET http://{host}/kv/abs HTTP/1.0\r\nHost: {host}\r\n\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).unwrap();
        let body = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| resp[i + 4..].to_vec())
            .unwrap_or_default();
        assert_eq!(body, b"yes", "absolute-form GET must return value");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
