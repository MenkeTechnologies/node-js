//! Node `tls` module: real TLS over blocking `rustls` (`rustls::StreamOwned`
//! wrapping a `std::net::TcpStream`).
//!
//! Threading model mirrors `net` (see `host::run_event_loop`): background threads
//! only move raw bytes and post `IoTask` closures onto the host channel; every
//! JS-visible effect (building the `TLSSocket`, emitting `secureConnect`/`data`/
//! `end`/`close`, calling listeners) happens on the main thread when the loop runs
//! the posted closure. Background closures NEVER capture a `Value` (the heap is a
//! main-thread `thread_local`); they capture only `Send` data (`u64` ids, byte
//! vectors, `TcpStream`s, channel senders) and look the emitter up by id inside
//! the posted `IoTask`.
//!
//! Per TLS connection there is ONE owner thread that solely owns the
//! `StreamOwned`. It reads with a short socket read-timeout (so a `WouldBlock`
//! lets it loop) and drains an mpsc channel of `WriteCmd`s produced by the main
//! thread (`socket.write`/`socket.end`). Because reads and writes share the one
//! rustls `Connection`, keeping both on a single thread avoids splitting the
//! stateful cipher across threads.

use crate::host::{invoke, with_host, IoTask, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use once_cell::sync::OnceCell;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, ConnectionCommon, DigitallySignedStruct, RootCertStore,
    ServerConfig, ServerConnection, SideData, SignatureScheme, StreamOwned,
};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

/// `tls` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &[
    "connect",
    "createServer",
    "createSecureContext",
    "checkServerIdentity",
    "convertALPNProtocols",
    "getCiphers",
    "getCACertificates",
    "setDefaultCACertificates",
    "getCertificateCompressionAlgorithms",
];

/// The standard OpenSSL cipher-suite names (lowercase) reported by
/// `tls.getCiphers()`, matching Node v26's fixed list (TLS 1.2 suites plus the
/// `tls_*` TLS 1.3 suites). A fixed protocol constant, not a runtime value.
const CIPHERS: &[&str] = &[
    "aes128-gcm-sha256",
    "aes128-sha",
    "aes128-sha256",
    "aes256-gcm-sha384",
    "aes256-sha",
    "aes256-sha256",
    "dhe-psk-aes128-cbc-sha",
    "dhe-psk-aes128-cbc-sha256",
    "dhe-psk-aes128-gcm-sha256",
    "dhe-psk-aes256-cbc-sha",
    "dhe-psk-aes256-cbc-sha384",
    "dhe-psk-aes256-gcm-sha384",
    "dhe-psk-chacha20-poly1305",
    "dhe-rsa-aes128-gcm-sha256",
    "dhe-rsa-aes128-sha",
    "dhe-rsa-aes128-sha256",
    "dhe-rsa-aes256-gcm-sha384",
    "dhe-rsa-aes256-sha",
    "dhe-rsa-aes256-sha256",
    "dhe-rsa-chacha20-poly1305",
    "ecdhe-ecdsa-aes128-gcm-sha256",
    "ecdhe-ecdsa-aes128-sha",
    "ecdhe-ecdsa-aes128-sha256",
    "ecdhe-ecdsa-aes256-gcm-sha384",
    "ecdhe-ecdsa-aes256-sha",
    "ecdhe-ecdsa-aes256-sha384",
    "ecdhe-ecdsa-chacha20-poly1305",
    "ecdhe-psk-aes128-cbc-sha",
    "ecdhe-psk-aes128-cbc-sha256",
    "ecdhe-psk-aes256-cbc-sha",
    "ecdhe-psk-aes256-cbc-sha384",
    "ecdhe-psk-chacha20-poly1305",
    "ecdhe-rsa-aes128-gcm-sha256",
    "ecdhe-rsa-aes128-sha",
    "ecdhe-rsa-aes128-sha256",
    "ecdhe-rsa-aes256-gcm-sha384",
    "ecdhe-rsa-aes256-sha",
    "ecdhe-rsa-aes256-sha384",
    "ecdhe-rsa-chacha20-poly1305",
    "psk-aes128-cbc-sha",
    "psk-aes128-cbc-sha256",
    "psk-aes128-gcm-sha256",
    "psk-aes256-cbc-sha",
    "psk-aes256-cbc-sha384",
    "psk-aes256-gcm-sha384",
    "psk-chacha20-poly1305",
    "rsa-psk-aes128-cbc-sha",
    "rsa-psk-aes128-cbc-sha256",
    "rsa-psk-aes128-gcm-sha256",
    "rsa-psk-aes256-cbc-sha",
    "rsa-psk-aes256-cbc-sha384",
    "rsa-psk-aes256-gcm-sha384",
    "rsa-psk-chacha20-poly1305",
    "srp-aes-128-cbc-sha",
    "srp-aes-256-cbc-sha",
    "srp-rsa-aes-128-cbc-sha",
    "srp-rsa-aes-256-cbc-sha",
    "tls_aes_128_ccm_8_sha256",
    "tls_aes_128_ccm_sha256",
    "tls_aes_128_gcm_sha256",
    "tls_aes_256_gcm_sha384",
    "tls_chacha20_poly1305_sha256",
];

/// Instance method names for the two `@@native` tags this module owns, exposed to
/// `stdlib::instance_has_method` (property reads that yield a bound method).
pub const SERVER_METHODS: &[&str] = &["listen", "close", "address"];
pub const SOCKET_METHODS: &[&str] = &[
    "write",
    "end",
    "destroy",
    "setEncoding",
    "setKeepAlive",
    "setNoDelay",
    "setTimeout",
    "ref",
    "unref",
    "pause",
    "resume",
];

/// A native-thread hook run on the main thread for each freshly-handshaked
/// connection. Set by `https` to attach its request parser; `None` for a plain
/// `tls` server (which emits `secureConnect`/`connection` and calls its listener).
pub type ConnHook = std::rc::Rc<dyn Fn(&Value, &Value, u64) -> Result<(), String>>;

/// A write request handed from the main thread to a socket's owner thread.
enum WriteCmd {
    Data(Vec<u8>),
    Shutdown,
}

/// Process-global monotonic id source for TLS sockets. Unlike `net`, ids are
/// generated on background owner threads (before their main-thread record
/// exists), so a lock-free atomic is used instead of a main-thread counter.
static NEXT_TLS_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_SERVER_ID: AtomicU64 = AtomicU64::new(1);

fn next_tls_id() -> u64 {
    NEXT_TLS_ID.fetch_add(1, Ordering::Relaxed)
}
fn next_server_id() -> u64 {
    NEXT_SERVER_ID.fetch_add(1, Ordering::Relaxed)
}

/// Main-thread record for a listening TLS server.
struct TlsServerRec {
    emitter: Value,
    stop: Arc<AtomicBool>,
    conn_hook: Option<ConnHook>,
    /// Optional JS `connectionListener` (`(socket) => …`) for a plain tls server.
    listener: Option<Value>,
}

/// Main-thread record for a live TLS socket.
struct TlsSocketRec {
    emitter: Value,
    /// Queue of writes for the socket's owner thread.
    tx: Sender<WriteCmd>,
}

#[derive(Default)]
struct TlsState {
    servers: HashMap<u64, TlsServerRec>,
    sockets: HashMap<u64, TlsSocketRec>,
}

thread_local! {
    static TLS: std::cell::RefCell<TlsState> = std::cell::RefCell::new(TlsState::default());
    /// `ServerConfig`s built by `createServer` before `listen` assigns a server id
    /// (mirrors `net::PENDING_HOOKS`).
    static PENDING_CONFIGS: std::cell::RefCell<Vec<(Value, Arc<ServerConfig>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static PENDING_HOOKS: std::cell::RefCell<Vec<(Value, ConnHook)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// User-supplied default CA certificates (PEM strings) set via
    /// `setDefaultCACertificates`; read back by `getCACertificates('default')`.
    static DEFAULT_CA_CERTS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

// ── shared object / prop helpers (same shape as net) ─────────────────────────

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn set_prop(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), val);
        }
    });
}

fn u64_prop(recv: &Value, key: &str) -> Option<u64> {
    get_prop(recv, key).map(|v| with_host(|h| h.to_number(&v)) as u64)
}

/// Delegate the EventEmitter surface to `events`; `None` for a non-emitter method.
fn emitter_dispatch(recv: &Value, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "on"
        | "addListener"
        | "prependListener"
        | "once"
        | "prependOnceListener"
        | "emit"
        | "removeListener"
        | "off"
        | "removeAllListeners"
        | "listenerCount"
        | "eventNames"
        | "setMaxListeners"
        | "getMaxListeners"
        | "listeners" => Some(super::events::instance_call(recv, method, args.to_vec())),
        _ => None,
    }
}

/// Raw bytes of a `write`/`end` argument (Buffer bytes or a string's UTF-8),
/// shared with the option-reading path for `key`/`cert` Buffers.
fn value_bytes(v: Option<&Value>) -> Vec<u8> {
    let Some(v) = v else { return Vec::new() };
    let is_buffer =
        with_host(|h| matches!(h.get(v), Some(JsObj::Object(p)) if p.contains_key("@@bytes")));
    if is_buffer {
        return with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|b| h.get(b)) {
                Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        });
    }
    with_host(|h| h.str_of(v)).into_bytes()
}

// ── module entry ─────────────────────────────────────────────────────────────

/// `stdlib::call` entry for `tls.<method>`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "connect" => Some(connect(args)),
        "createServer" => Some(create_server(args)),
        // A secure context is just the parsed key/cert pair; we build the real
        // `ServerConfig` lazily in `createServer`, so expose an opaque holder.
        "createSecureContext" => Some(Ok(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("@@native".into(), h.new_str("SecureContext"));
            if let Some(o) = args.first() {
                m.insert("@@options".into(), o.clone());
            }
            h.new_object(m)
        }))),
        "checkServerIdentity" => Some(check_server_identity(args)),
        "convertALPNProtocols" => Some(convert_alpn_protocols(args)),
        "getCiphers" => Some(Ok(with_host(|h| {
            let items = CIPHERS.iter().map(|c| h.new_str(*c)).collect();
            h.new_array(items)
        }))),
        "getCACertificates" => Some(get_ca_certificates(args)),
        "setDefaultCACertificates" => Some(set_default_ca_certificates(args)),
        // rustls' default configuration does not negotiate TLS certificate
        // compression (RFC 8879), so no algorithm is actually supported. Node v26
        // likewise returns `[]` here.
        "getCertificateCompressionAlgorithms" => Some(Ok(with_host(|h| h.new_array(Vec::new())))),
        _ => None,
    }
}

// ── tls.checkServerIdentity / ALPN / CA certificates ─────────────────────────

/// Read an array of strings (or a single string) from a value; Buffers/other
/// element types are stringified. Empty when the value is absent or not iterable.
fn read_string_array(v: Option<&Value>) -> Vec<String> {
    let Some(v) = v else { return Vec::new() };
    with_host(|h| match h.get(v) {
        Some(JsObj::Array(items)) => items.iter().map(|x| h.str_of(x)).collect(),
        _ if h.as_str(v).is_some() => vec![h.str_of(v)],
        _ => Vec::new(),
    })
}

/// Does `host` match the certificate identity `pattern`, honouring a single
/// leftmost-label wildcard (`*.example.com` matches `a.example.com` but not
/// `example.com` nor `a.b.example.com`)? Case-insensitive.
fn host_matches(host: &str, pattern: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let pattern = pattern.trim().to_ascii_lowercase();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        // Wildcard covers exactly one leftmost label.
        match host.split_once('.') {
            Some((_, rest)) => rest == suffix,
            None => false,
        }
    } else {
        host == pattern
    }
}

/// `tls.checkServerIdentity(hostname, cert)` — best-effort string comparison of
/// `hostname` against the certificate's `subjectaltname` DNS entries (falling
/// back to `subject.CN` when no SAN is present). Returns `undefined` on a match,
/// or an `ERR_TLS_CERT_ALTNAME_INVALID`-shaped Error object on mismatch.
fn check_server_identity(args: &[Value]) -> Result<Value, String> {
    let host = super::arg_str(args, 0);
    let cert = args.get(1).cloned().unwrap_or(Value::Undef);

    // Collect DNS altnames from `subjectaltname` ("DNS:a.com, DNS:*.b.com").
    let altname_str = get_prop(&cert, "subjectaltname")
        .filter(|v| with_host(|h| h.as_str(v)).is_some())
        .map(|v| with_host(|h| h.str_of(&v)))
        .unwrap_or_default();
    let dns_names: Vec<String> = altname_str
        .split(',')
        .filter_map(|e| e.trim().strip_prefix("DNS:").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();

    let matched = if !dns_names.is_empty() {
        dns_names.iter().any(|p| host_matches(&host, p))
    } else {
        // No SAN: fall back to the subject Common Name.
        let cn = get_prop(&cert, "subject")
            .and_then(|s| get_prop(&s, "CN"))
            .filter(|v| with_host(|h| h.as_str(v)).is_some())
            .map(|v| with_host(|h| h.str_of(&v)))
            .unwrap_or_default();
        !cn.is_empty() && host_matches(&host, &cn)
    };

    if matched {
        return Ok(Value::Undef);
    }
    let reason = if !altname_str.is_empty() {
        format!("Host: {host}. is not in the cert's altnames: {altname_str}")
    } else {
        format!("Host: {host}. is not cert's CN")
    };
    let message = format!("Hostname/IP does not match certificate's altnames: {reason}");
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("message".into(), h.new_str(message));
        m.insert("reason".into(), h.new_str(reason));
        m.insert("host".into(), h.new_str(host));
        m.insert("code".into(), h.new_str("ERR_TLS_CERT_ALTNAME_INVALID"));
        m.insert("cert".into(), cert);
        h.new_object(m)
    }))
}

/// `tls.convertALPNProtocols(protocols, out)` — encode `protocols` into the wire
/// format (each entry prefixed by a single length byte), store it on
/// `out.ALPNProtocols` as a Buffer, and return that Buffer.
fn convert_alpn_protocols(args: &[Value]) -> Result<Value, String> {
    let protocols = read_string_array(args.first());
    let mut wire = Vec::new();
    for p in &protocols {
        let bytes = p.as_bytes();
        // Node clamps to 255 (a single length byte); over-long entries throw, but
        // best-effort here simply truncates the encoded length.
        wire.push(bytes.len().min(255) as u8);
        wire.extend_from_slice(&bytes[..bytes.len().min(255)]);
    }
    let buf = super::buffer::from_bytes(&wire);
    if let Some(out) = args.get(1).filter(|v| matches!(v, Value::Obj(_))) {
        set_prop(out, "ALPNProtocols", buf.clone());
    }
    Ok(buf)
}

/// `tls.getCACertificates([type])` — return PEM strings for the requested store.
///
/// LIMITATION: the bundled `'default'` Mozilla store cannot be reproduced from
/// `webpki-roots`, which ships extracted *trust anchors* (subject + SPKI + name
/// constraints), not full X.509 certificates — so no faithful PEM can be built
/// from them. `'default'` therefore returns whatever was installed via
/// `setDefaultCACertificates` (empty until then); `'system'`/`'extra'`/`'bundled'`
/// return `[]`.
fn get_ca_certificates(args: &[Value]) -> Result<Value, String> {
    let kind = if args.is_empty() {
        "default".to_string()
    } else {
        super::arg_str(args, 0)
    };
    let certs: Vec<String> = match kind.as_str() {
        "default" => DEFAULT_CA_CERTS.with(|c| c.borrow().clone()),
        _ => Vec::new(),
    };
    Ok(with_host(|h| {
        let items = certs.into_iter().map(|c| h.new_str(c)).collect();
        h.new_array(items)
    }))
}

/// `tls.setDefaultCACertificates(certs)` — store `certs` (PEM strings or Buffers)
/// as the process default CA set for later `getCACertificates('default')` reads.
fn set_default_ca_certificates(args: &[Value]) -> Result<Value, String> {
    let certs = read_string_array(args.first());
    DEFAULT_CA_CERTS.with(|c| *c.borrow_mut() = certs);
    Ok(Value::Undef)
}

// ── TLS crypto config ────────────────────────────────────────────────────────

/// The shared verifying client config (explicit aws-lc-rs crypto provider +
/// webpki-roots trust anchors). Built once. Uses `builder_with_provider` rather
/// than `builder()` so it never depends on a process-default provider being
/// pre-installed (the insecure path installs none).
fn verifying_client_config() -> Arc<ClientConfig> {
    static CFG: OnceCell<Arc<ClientConfig>> = OnceCell::new();
    CFG.get_or_init(|| {
        let root_store = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("aws-lc-rs provider supports the default protocol versions")
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Arc::new(cfg)
    })
    .clone()
}

/// A client config that accepts any server certificate (`rejectUnauthorized:
/// false`). Signature verification is still delegated to the default provider's
/// algorithms; only chain-to-a-trust-anchor and hostname checks are skipped.
fn insecure_client_config() -> Arc<ClientConfig> {
    static CFG: OnceCell<Arc<ClientConfig>> = OnceCell::new();
    CFG.get_or_init(|| {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let cfg = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("aws-lc-rs provider supports the default protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth();
        Arc::new(cfg)
    })
    .clone()
}

/// A `ServerCertVerifier` that skips certificate-chain/hostname validation but
/// still checks the handshake signature against the default provider's algorithms.
#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The shared client `ClientConfig` for a given `rejectUnauthorized` setting.
/// Public so the `https` client (`https.request`/`https.get`) reuses the same
/// trust configuration as `tls.connect`.
pub fn client_config(reject_unauthorized: bool) -> Arc<ClientConfig> {
    if reject_unauthorized {
        verifying_client_config()
    } else {
        insecure_client_config()
    }
}

/// Build a `ServerConfig` from PEM `key`+`cert` bytes.
pub fn build_server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<ServerConfig>, String> {
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &cert_pem[..])
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Error: tls: bad certificate PEM: {e}"))?;
    if certs.is_empty() {
        return Err("Error: tls: no certificates found in `cert`".to_string());
    }
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut &key_pem[..])
        .map_err(|e| format!("Error: tls: bad private key PEM: {e}"))?
        .ok_or_else(|| "Error: tls: no private key found in `key`".to_string())?;
    let cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("Error: tls: invalid key/cert: {e}"))?;
    Ok(Arc::new(cfg))
}

// ── tls.connect (client) ─────────────────────────────────────────────────────

/// `tls.connect(options[, cb])` / `tls.connect(port[, host][, options][, cb])`.
/// Returns a `TLSSocket` immediately; the TCP connect + handshake run on a
/// background thread and emit `secureConnect` (or `error`) when complete.
pub fn connect(args: &[Value]) -> Result<Value, String> {
    let mut port: u16 = 0;
    let mut host = "localhost".to_string();
    let mut servername: Option<String> = None;
    let mut reject_unauthorized = true;
    let mut cb: Option<Value> = None;

    for a in args {
        let n = with_host(|h| h.to_number(a));
        if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        } else if !n.is_nan() && matches!(a, Value::Float(_) | Value::Int(_)) {
            port = n as u16;
        } else if with_host(|h| h.as_str(a)).is_some() {
            host = with_host(|h| h.str_of(a));
        } else if matches!(a, Value::Obj(_)) {
            // Options object.
            if let Some(p) = get_prop(a, "port") {
                port = with_host(|h| h.to_number(&p)) as u16;
            }
            for key in ["host", "hostname"] {
                if let Some(hv) = get_prop(a, key).filter(|v| with_host(|h| h.as_str(v)).is_some())
                {
                    host = with_host(|h| h.str_of(&hv));
                }
            }
            if let Some(sv) =
                get_prop(a, "servername").filter(|v| with_host(|h| h.as_str(v)).is_some())
            {
                servername = Some(with_host(|h| h.str_of(&sv)));
            }
            if let Some(rv) = get_prop(a, "rejectUnauthorized") {
                reject_unauthorized = with_host(|h| h.truthy(&rv));
            }
        }
    }
    let servername = servername.unwrap_or_else(|| host.clone());

    // Build the socket object + register its write channel on the main thread.
    let sock_id = next_tls_id();
    let (tx, rx) = std::sync::mpsc::channel::<WriteCmd>();
    let mut extra = IndexMap::new();
    extra.insert("@@tlsid".into(), Value::Float(sock_id as f64));
    extra.insert("authorized".into(), Value::Bool(reject_unauthorized));
    extra.insert("encrypted".into(), Value::Bool(true));
    let socket = new_emitter_object("TLSSocket", extra);
    TLS.with(|s| {
        s.borrow_mut().sockets.insert(
            sock_id,
            TlsSocketRec {
                emitter: socket.clone(),
                tx,
            },
        );
    });
    with_host(|h| h.incr_handle());
    // `tls.connect(opts, cb)` registers `cb` as a one-shot `secureConnect` listener.
    if let Some(cb) = cb {
        super::events::instance_call(
            &socket,
            "once",
            vec![with_host(|h| h.new_str("secureConnect")), cb],
        )?;
    }

    let io_tx = with_host(|h| h.io_sender());
    let config = if reject_unauthorized {
        verifying_client_config()
    } else {
        insecure_client_config()
    };

    std::thread::spawn(move || {
        let server_name = match ServerName::try_from(servername.clone()) {
            Ok(n) => n,
            Err(_) => {
                post_socket_error(
                    &io_tx,
                    sock_id,
                    format!("Error: tls: invalid servername '{servername}'"),
                );
                return;
            }
        };
        let mut sock = match TcpStream::connect((host.as_str(), port)) {
            Ok(s) => s,
            Err(e) => {
                post_socket_error(
                    &io_tx,
                    sock_id,
                    format!("Error: connect ECONNREFUSED {host}:{port}: {e}"),
                );
                return;
            }
        };
        let mut conn = match ClientConnection::new(config, server_name) {
            Ok(c) => c,
            Err(e) => {
                post_socket_error(&io_tx, sock_id, format!("Error: tls: {e}"));
                return;
            }
        };
        // Drive the handshake to completion (blocking).
        if let Err(e) = conn.complete_io(&mut sock) {
            post_socket_error(&io_tx, sock_id, format!("Error: tls handshake failed: {e}"));
            return;
        }
        let _ = io_tx.send(Box::new(move || on_secure_connect(sock_id)));
        let stream = StreamOwned::new(conn, sock);
        owner_loop(stream, sock_id, rx, io_tx);
    });

    Ok(socket)
}

/// Emit `secureConnect` on a freshly-handshaked socket (runs on the main thread).
fn on_secure_connect(sock_id: u64) -> Result<(), String> {
    let socket = TLS.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.emitter.clone()));
    if let Some(socket) = socket {
        super::events::instance_call(
            &socket,
            "emit",
            vec![with_host(|h| h.new_str("secureConnect"))],
        )?;
    }
    Ok(())
}

/// Post an `error` event (then close) for a socket whose connect/handshake failed.
fn post_socket_error(io_tx: &Sender<IoTask>, sock_id: u64, msg: String) {
    let _ = io_tx.send(Box::new(move || {
        let socket = TLS.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.emitter.clone()));
        if let Some(socket) = socket {
            let err = with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("message".into(), h.new_str(msg.clone()));
                h.new_object(m)
            });
            super::events::instance_call(
                &socket,
                "emit",
                vec![with_host(|h| h.new_str("error")), err],
            )?;
        }
        on_socket_close(sock_id)
    }));
}

// ── tls.createServer ─────────────────────────────────────────────────────────

/// `tls.createServer([options][, secureConnectionListener])`. Parses `key`+`cert`
/// into a `ServerConfig` eagerly (so a bad cert throws synchronously) and returns
/// a `TLSServer` emitter.
pub fn create_server(args: &[Value]) -> Result<Value, String> {
    let mut options: Option<Value> = None;
    let mut listener: Option<Value> = None;
    for a in args {
        if with_host(|h| crate::host::is_callable(h, a)) {
            listener = Some(a.clone());
        } else if matches!(a, Value::Obj(_)) {
            options = Some(a.clone());
        }
    }
    let opts = options.ok_or_else(|| {
        crate::host::type_error("tls.createServer requires an options object with `key` and `cert`")
    })?;
    let cert = value_bytes(get_prop(&opts, "cert").as_ref());
    let key = value_bytes(get_prop(&opts, "key").as_ref());
    if cert.is_empty() || key.is_empty() {
        return Err(crate::host::type_error(
            "tls.createServer requires `key` and `cert`",
        ));
    }
    let config = build_server_config(&cert, &key)?;

    let mut extra = IndexMap::new();
    if let Some(cb) = listener {
        extra.insert("@@connListener".into(), cb);
    }
    let server = new_emitter_object("TLSServer", extra);
    PENDING_CONFIGS.with(|p| p.borrow_mut().push((server.clone(), config)));
    Ok(server)
}

/// Build a TLS server backed by a caller-supplied per-connection hook (used by
/// `https::create_server`). Returns the `TLSServer` emitter; the caller stores its
/// `requestListener` and registers the hook.
pub fn create_server_with_config(
    config: Arc<ServerConfig>,
    hook: ConnHook,
    request_listener: Value,
) -> Value {
    let mut extra = IndexMap::new();
    extra.insert("@@requestListener".into(), request_listener);
    let server = new_emitter_object("TLSServer", extra);
    PENDING_CONFIGS.with(|p| p.borrow_mut().push((server.clone(), config)));
    PENDING_HOOKS.with(|p| p.borrow_mut().push((server.clone(), hook)));
    server
}

fn take_pending_config(server: &Value) -> Option<Arc<ServerConfig>> {
    PENDING_CONFIGS.with(|p| {
        let mut p = p.borrow_mut();
        p.iter()
            .position(|(s, _)| s == server)
            .map(|pos| p.remove(pos).1)
    })
}
fn take_pending_hook(server: &Value) -> Option<ConnHook> {
    PENDING_HOOKS.with(|p| {
        let mut p = p.borrow_mut();
        p.iter()
            .position(|(s, _)| s == server)
            .map(|pos| p.remove(pos).1)
    })
}

// ── instance dispatch (TLSServer / TLSSocket) ────────────────────────────────

pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match tag {
        "TLSServer" => server_call(recv, method, args),
        "TLSSocket" => socket_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

fn server_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "listen" => server_listen(recv, &args),
        "close" => server_close(recv, &args),
        "address" => Ok(get_prop(recv, "@@address").unwrap_or(Value::Undef)),
        _ => Err(crate::host::type_error(&format!(
            "server.{method} is not a function"
        ))),
    }
}

/// `server.listen(port[, host][, callback])`. Binds on the main thread, spawns the
/// accept loop, and fires `listening` + callback asynchronously.
fn server_listen(recv: &Value, args: &[Value]) -> Result<Value, String> {
    let port = with_host(|h| args.first().map(|v| h.to_number(v)).unwrap_or(0.0)) as u16;
    let mut host = "0.0.0.0".to_string();
    let mut cb: Option<Value> = None;
    for a in &args[1.min(args.len())..] {
        if with_host(|h| h.as_str(a)).is_some() {
            host = with_host(|h| h.str_of(a));
        } else if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        }
    }

    let config = take_pending_config(recv)
        .ok_or_else(|| crate::host::type_error("tls server has no secure context"))?;
    let listener = std::net::TcpListener::bind((host.as_str(), port))
        .map_err(|e| format!("Error: listen EADDRINUSE: {e}"))?;
    let local = listener.local_addr().ok();

    let id = next_server_id();
    set_prop(recv, "@@serverid", Value::Float(id as f64));
    if let Some(addr) = local {
        let mut a = IndexMap::new();
        a.insert("port".into(), Value::Float(addr.port() as f64));
        a.insert(
            "address".into(),
            with_host(|h| h.new_str(addr.ip().to_string())),
        );
        a.insert(
            "family".into(),
            with_host(|h| h.new_str(if addr.is_ipv6() { "IPv6" } else { "IPv4" })),
        );
        let addr_obj = with_host(|h| h.new_object(a));
        set_prop(recv, "@@address", addr_obj);
    }
    let conn_hook = take_pending_hook(recv);
    let request_listener = get_prop(recv, "@@requestListener");
    let plain_listener = get_prop(recv, "@@connListener");
    let stop = Arc::new(AtomicBool::new(false));
    TLS.with(|s| {
        s.borrow_mut().servers.insert(
            id,
            TlsServerRec {
                emitter: recv.clone(),
                stop: stop.clone(),
                conn_hook,
                listener: plain_listener,
            },
        );
    });
    // Preserve the https request listener where the hook can find it (already on
    // the object as `@@requestListener`); nothing else to stash.
    let _ = request_listener;
    with_host(|h| h.incr_handle());

    let io_tx = with_host(|h| h.io_sender());
    listener.set_nonblocking(true).ok();
    std::thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            match listener.accept() {
                Ok((stream, _addr)) => {
                    let cfg = config.clone();
                    let tx = io_tx.clone();
                    // One handshake+owner thread per connection.
                    std::thread::spawn(move || accept_connection(id, stream, cfg, tx));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    let server = recv.clone();
    let _ = with_host(|h| h.io_sender()).send(Box::new(move || {
        super::events::instance_call(&server, "emit", vec![with_host(|h| h.new_str("listening"))])?;
        if let Some(cb) = cb {
            invoke(&cb, Vec::new(), None)?;
        }
        Ok(())
    }));
    Ok(recv.clone())
}

/// A background per-connection thread: complete the server handshake, then post an
/// `IoTask` that surfaces the socket to JS, and finally become its owner loop.
fn accept_connection(
    server_id: u64,
    mut sock: TcpStream,
    config: Arc<ServerConfig>,
    io_tx: Sender<IoTask>,
) {
    let mut conn = match ServerConnection::new(config) {
        Ok(c) => c,
        Err(_) => return,
    };
    if conn.complete_io(&mut sock).is_err() {
        return;
    }
    let sock_id = next_tls_id();
    let (tx, rx) = std::sync::mpsc::channel::<WriteCmd>();
    let tx_for_main = tx;
    let _ = io_tx.send(Box::new(move || {
        on_server_connection(server_id, sock_id, tx_for_main)
    }));
    let stream = StreamOwned::new(conn, sock);
    owner_loop(stream, sock_id, rx, io_tx);
}

/// Main-thread handler for a newly-handshaked server connection: build the
/// `TLSSocket`, register it, run the server's hook/listener, emit events.
fn on_server_connection(server_id: u64, sock_id: u64, tx: Sender<WriteCmd>) -> Result<(), String> {
    let server = TLS.with(|s| {
        s.borrow()
            .servers
            .get(&server_id)
            .map(|r| r.emitter.clone())
    });
    let Some(server) = server else { return Ok(()) };

    let mut extra = IndexMap::new();
    extra.insert("@@tlsid".into(), Value::Float(sock_id as f64));
    extra.insert("encrypted".into(), Value::Bool(true));
    let socket = new_emitter_object("TLSSocket", extra);
    TLS.with(|s| {
        s.borrow_mut().sockets.insert(
            sock_id,
            TlsSocketRec {
                emitter: socket.clone(),
                tx,
            },
        );
    });
    with_host(|h| h.incr_handle());

    // `secureConnection` is the tls server event; also emit `connection` for parity.
    super::events::instance_call(
        &server,
        "emit",
        vec![with_host(|h| h.new_str("secureConnection")), socket.clone()],
    )?;
    super::events::instance_call(
        &server,
        "emit",
        vec![with_host(|h| h.new_str("connection")), socket.clone()],
    )?;

    // https attaches a request-parser hook; a plain tls server runs its listener.
    let hook = TLS.with(|s| {
        s.borrow()
            .servers
            .get(&server_id)
            .and_then(|r| r.conn_hook.clone())
    });
    if let Some(hook) = hook {
        hook(&server, &socket, sock_id)?;
    } else {
        let listener = TLS.with(|s| {
            s.borrow()
                .servers
                .get(&server_id)
                .and_then(|r| r.listener.clone())
        });
        if let Some(cb) = listener {
            invoke(&cb, vec![socket.clone()], None)?;
        }
    }
    Ok(())
}

fn server_close(recv: &Value, args: &[Value]) -> Result<Value, String> {
    if let Some(id) = u64_prop(recv, "@@serverid") {
        let rec = TLS.with(|s| s.borrow_mut().servers.remove(&id));
        if let Some(rec) = rec {
            rec.stop.store(true, Ordering::Release);
            with_host(|h| h.decr_handle());
            let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
        }
    }
    if let Some(cb) = args
        .first()
        .filter(|v| with_host(|h| crate::host::is_callable(h, v)))
    {
        invoke(cb, Vec::new(), None)?;
    }
    super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))])?;
    Ok(recv.clone())
}

// ── TLSSocket instance methods ───────────────────────────────────────────────

fn socket_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "write" => {
            if let Some(id) = u64_prop(recv, "@@tlsid") {
                socket_write(id, &value_bytes(args.first()));
            }
            Ok(Value::Bool(true))
        }
        "end" => {
            if let Some(id) = u64_prop(recv, "@@tlsid") {
                if let Some(chunk) = args.first().filter(|v| !matches!(v, Value::Undef)) {
                    socket_write(id, &value_bytes(Some(chunk)));
                }
                socket_end(id);
            }
            Ok(recv.clone())
        }
        "destroy" => {
            if let Some(id) = u64_prop(recv, "@@tlsid") {
                socket_end(id);
            }
            Ok(recv.clone())
        }
        "setEncoding" | "setTimeout" | "setNoDelay" | "setKeepAlive" | "ref" | "unref"
        | "pause" | "resume" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "socket.{method} is not a function"
        ))),
    }
}

/// Queue plaintext to be written by the socket's owner thread (no-op if closed).
/// Public so `https` server responses can write through the TLS channel.
pub fn socket_write(sock_id: u64, data: &[u8]) {
    let tx = TLS.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.tx.clone()));
    if let Some(tx) = tx {
        let _ = tx.send(WriteCmd::Data(data.to_vec()));
    }
}

/// Signal end-of-write (TLS close-notify + TCP write shutdown) on a socket.
pub fn socket_end(sock_id: u64) {
    let tx = TLS.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.tx.clone()));
    if let Some(tx) = tx {
        let _ = tx.send(WriteCmd::Shutdown);
    }
}

// ── the per-socket owner thread ──────────────────────────────────────────────

/// Sole owner of one connection's `StreamOwned`. Reads with a short socket
/// read-timeout (a `WouldBlock` just loops) and drains the write channel. Posts
/// `data`/`end`/`close` `IoTask`s to the main thread. Generic over client/server
/// connections (both deref to `ConnectionCommon`), mirroring `StreamOwned`'s
/// bounds.
fn owner_loop<C, S>(
    mut stream: StreamOwned<C, TcpStream>,
    sock_id: u64,
    rx: Receiver<WriteCmd>,
    io_tx: Sender<IoTask>,
) where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
{
    stream
        .sock
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .ok();
    let mut buf = [0u8; 16384];
    loop {
        // 1) drain any queued writes.
        let mut shutdown = false;
        loop {
            match rx.try_recv() {
                Ok(WriteCmd::Data(bytes)) => {
                    if stream
                        .write_all(&bytes)
                        .and_then(|_| stream.flush())
                        .is_err()
                    {
                        let _ = io_tx.send(Box::new(move || on_socket_close(sock_id)));
                        return;
                    }
                }
                Ok(WriteCmd::Shutdown) => shutdown = true,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        if shutdown {
            stream.conn.send_close_notify();
            let _ = stream.flush();
            let _ = stream.sock.shutdown(std::net::Shutdown::Write);
        }

        // 2) read whatever plaintext is available.
        match stream.read(&mut buf) {
            Ok(0) => {
                let _ = io_tx.send(Box::new(move || on_socket_end(sock_id)));
                return;
            }
            Ok(n) => {
                let bytes = buf[..n].to_vec();
                let _ = io_tx.send(Box::new(move || on_socket_data(sock_id, bytes)));
            }
            Err(ref e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // No plaintext ready; loop to service writes again.
                continue;
            }
            Err(_) => {
                let _ = io_tx.send(Box::new(move || on_socket_close(sock_id)));
                return;
            }
        }
    }
}

// ── main-thread socket event handlers (run from posted IoTasks) ──────────────

fn on_socket_data(sock_id: u64, bytes: Vec<u8>) -> Result<(), String> {
    let socket = TLS.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.emitter.clone()));
    let Some(socket) = socket else { return Ok(()) };
    // Feed the https request parser first (no-op unless this is an https conn).
    super::https::feed(sock_id, &socket, &bytes)?;
    let chunk = super::buffer::from_bytes(&bytes);
    super::events::instance_call(
        &socket,
        "emit",
        vec![with_host(|h| h.new_str("data")), chunk],
    )?;
    Ok(())
}

fn on_socket_end(sock_id: u64) -> Result<(), String> {
    let socket = TLS.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.emitter.clone()));
    if let Some(socket) = socket {
        super::events::instance_call(&socket, "emit", vec![with_host(|h| h.new_str("end"))])?;
    }
    on_socket_close(sock_id)
}

fn on_socket_close(sock_id: u64) -> Result<(), String> {
    let rec = TLS.with(|s| s.borrow_mut().sockets.remove(&sock_id));
    super::https::drop_conn(sock_id);
    if let Some(rec) = rec {
        super::events::instance_call(
            &rec.emitter,
            "emit",
            vec![with_host(|h| h.new_str("close"))],
        )?;
        with_host(|h| h.decr_handle());
        let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    }
    Ok(())
}

// ── shared emitter constructor (same shape as net) ───────────────────────────

/// Build a native emitter object (`@@native` tag + `@@on`/`@@once` maps + extras),
/// sharing the EventEmitter shape with `events`/`net`.
pub fn new_emitter_object(tag: &str, mut extra: IndexMap<String, Value>) -> Value {
    with_host(|h| {
        let on = h.new_object(IndexMap::new());
        let once = h.new_object(IndexMap::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@on".into(), on);
        m.insert("@@once".into(), once);
        for (k, v) in extra.drain(..) {
            m.insert(k, v);
        }
        h.new_object(m)
    })
}
