//! Optional TLS 1.3 for MTCP (RFC-0021 / RFC-0050 P0.5).
//!
//! Without `--features tls` this module is a pass-through (cleartext lab).

use std::io::{Read, Write};
use std::net::TcpStream;

use crate::{Result, StoreError};

/// Read+Write session (plain TCP or rustls). Separate traits cannot share one
/// `dyn` object; this glue is the MTCP I/O surface.
pub trait SessionIo: Read + Write + Send {}
impl<T: Read + Write + Send> SessionIo for T {}

/// Framed MTCP session (plain TCP or rustls).
pub type IoBox = Box<dyn SessionIo>;

/// Wrap a client TCP stream if process TLS is installed.
///
/// # Errors
/// Handshake / rustls.
pub fn maybe_client_wrap(s: TcpStream) -> Result<IoBox> {
    #[cfg(feature = "tls")]
    {
        if let Some(tls) = CLIENT.get() {
            return wrap_client(s, tls);
        }
    }
    Ok(Box::new(s))
}

/// Wrap an accepted TCP stream if process TLS is installed.
///
/// # Errors
/// Handshake / rustls.
pub fn maybe_server_wrap(s: TcpStream) -> Result<IoBox> {
    #[cfg(feature = "tls")]
    {
        if let Some(cfg) = SERVER.get() {
            return wrap_server(s, cfg.clone());
        }
    }
    Ok(Box::new(s))
}

/// Whether this process installed TLS (server or client).
#[must_use]
pub fn tls_installed() -> bool {
    #[cfg(feature = "tls")]
    {
        SERVER.get().is_some() || CLIENT.get().is_some()
    }
    #[cfg(not(feature = "tls"))]
    {
        false
    }
}

#[cfg(feature = "tls")]
use std::path::Path;
#[cfg(feature = "tls")]
use std::sync::{Arc, OnceLock};

#[cfg(feature = "tls")]
static SERVER: OnceLock<Arc<rustls::ServerConfig>> = OnceLock::new();
#[cfg(feature = "tls")]
static CLIENT: OnceLock<ClientTls> = OnceLock::new();

#[cfg(feature = "tls")]
struct ClientTls {
    config: Arc<rustls::ClientConfig>,
    server_name: String,
}

/// Load PEMs and install process-wide mTLS (server + client).
///
/// `ca` is both the trust root and the client-auth verifier (mTLS).
///
/// # Errors
/// File I/O / PEM / rustls config / already installed.
#[cfg(feature = "tls")]
pub fn install_from_pem_files(
    cert: &Path,
    key: &Path,
    ca: &Path,
    server_name: &str,
) -> Result<()> {
    let certs = load_certs(cert)?;
    let key = load_key(key)?;
    let roots = load_roots(ca)?;

    let verifier = rustls::server::AllowAnyAuthenticatedClient::new(roots.clone());
    let server = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_client_cert_verifier(Arc::new(verifier))
        .with_single_cert(certs.clone(), key.clone())
        .map_err(|e| StoreError::Msg(format!("tls server cert: {e}")))?;

    let client = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(roots)
        .with_client_auth_cert(certs, key)
        .map_err(|e| StoreError::Msg(format!("tls client cert: {e}")))?;

    SERVER
        .set(Arc::new(server))
        .map_err(|_| StoreError::Msg("tls server already installed".into()))?;
    CLIENT
        .set(ClientTls {
            config: Arc::new(client),
            server_name: server_name.to_string(),
        })
        .map_err(|_| StoreError::Msg("tls client already installed".into()))?;
    Ok(())
}

/// Stub when the crate is built without `--features tls`.
///
/// # Errors
/// Always — rebuild with the `tls` feature to load PEMs.
#[cfg(not(feature = "tls"))]
pub fn install_from_pem_files(
    _cert: &std::path::Path,
    _key: &std::path::Path,
    _ca: &std::path::Path,
    _server_name: &str,
) -> Result<()> {
    Err(StoreError::Msg(
        "montanha-tcp TLS requires building with --features tls".into(),
    ))
}

#[cfg(feature = "tls")]
fn wrap_server(s: TcpStream, cfg: Arc<rustls::ServerConfig>) -> Result<IoBox> {
    let conn = rustls::ServerConnection::new(cfg)
        .map_err(|e| StoreError::Msg(format!("tls server conn: {e}")))?;
    Ok(Box::new(rustls::StreamOwned::new(conn, s)))
}

#[cfg(feature = "tls")]
fn wrap_client(s: TcpStream, tls: &ClientTls) -> Result<IoBox> {
    let name = rustls::ServerName::try_from(tls.server_name.as_str())
        .map_err(|_| StoreError::Msg(format!("tls bad server name {}", tls.server_name)))?;
    let conn = rustls::ClientConnection::new(Arc::clone(&tls.config), name)
        .map_err(|e| StoreError::Msg(format!("tls client conn: {e}")))?;
    Ok(Box::new(rustls::StreamOwned::new(conn, s)))
}

#[cfg(feature = "tls")]
fn load_certs(path: &Path) -> Result<Vec<rustls::Certificate>> {
    let f = std::fs::File::open(path)
        .map_err(|e| StoreError::Msg(format!("tls cert {}: {e}", path.display())))?;
    let mut r = std::io::BufReader::new(f);
    let der = rustls_pemfile::certs(&mut r)
        .map_err(|e| StoreError::Msg(format!("tls cert pem: {e}")))?;
    if der.is_empty() {
        return Err(StoreError::Msg(format!(
            "tls cert {}: no certificates",
            path.display()
        )));
    }
    Ok(der.into_iter().map(rustls::Certificate).collect())
}

#[cfg(feature = "tls")]
fn load_key(path: &Path) -> Result<rustls::PrivateKey> {
    let f = std::fs::File::open(path)
        .map_err(|e| StoreError::Msg(format!("tls key {}: {e}", path.display())))?;
    let mut r = std::io::BufReader::new(f);
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut r)
        .map_err(|e| StoreError::Msg(format!("tls key pem: {e}")))?;
    if keys.is_empty() {
        return Err(StoreError::Msg(format!(
            "tls key {}: no PKCS#8 key",
            path.display()
        )));
    }
    Ok(rustls::PrivateKey(keys.remove(0)))
}

#[cfg(feature = "tls")]
fn load_roots(path: &Path) -> Result<rustls::RootCertStore> {
    let certs = load_certs(path)?;
    let mut roots = rustls::RootCertStore::empty();
    for c in certs {
        roots
            .add(&c)
            .map_err(|e| StoreError::Msg(format!("tls ca: {e}")))?;
    }
    Ok(roots)
}
