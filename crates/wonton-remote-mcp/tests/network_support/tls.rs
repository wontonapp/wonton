use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::PrivatePkcs8KeyDer;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::{LazyConfigAcceptor, TlsAcceptor};

use super::Server;

#[derive(Clone, Copy)]
pub enum ProxyMode {
    Direct,
    Http,
    Https,
    WrongCertificate,
}

#[derive(Clone, Copy)]
pub enum TargetMode {
    Eof,
    WrongCertificate,
    Healthy,
}

#[derive(Debug, Default)]
pub struct Trace {
    pub proxy_tls_accepted: bool,
    pub proxy_tls_rejected: bool,
    pub connect_request: Option<String>,
    pub target_sni: Option<String>,
    pub target_tls_accepted: bool,
    pub target_tls_rejected: bool,
    pub target_request: Option<String>,
}

pub struct TlsFixture {
    pub server: Server,
    pub ca_file: PathBuf,
    traces: mpsc::UnboundedReceiver<Trace>,
}

impl TlsFixture {
    pub async fn start(proxy_mode: ProxyMode, target_mode: TargetMode) -> Self {
        let authority = Authority::new();
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let directory = std::env::temp_dir().join(format!("wonton-network-tls-{random:02x?}"));
        std::fs::create_dir(&directory).unwrap();
        let ca_file = directory.join("ca.pem");
        std::fs::write(&ca_file, authority.certificate.pem()).unwrap();
        let proxy_tls = match proxy_mode {
            ProxyMode::Direct | ProxyMode::Http => None,
            ProxyMode::Https => Some(authority.server_config("127.0.0.1")),
            ProxyMode::WrongCertificate => Some(authority.server_config("wrong-proxy.example")),
        };
        let target_tls = authority.server_config(match target_mode {
            TargetMode::WrongCertificate => "wrong-target.example",
            TargetMode::Eof | TargetMode::Healthy => "tls-target.example",
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let accepted = connections.clone();
        let (sender, traces) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                accepted.fetch_add(1, Ordering::SeqCst);
                let trace = connection(
                    socket,
                    proxy_mode,
                    proxy_tls.clone(),
                    target_mode,
                    target_tls.clone(),
                )
                .await;
                if sender.send(trace).is_err() {
                    break;
                }
            }
        });
        Self {
            server: Server {
                address,
                connections,
                task,
            },
            ca_file,
            traces,
        }
    }

    pub async fn trace(&mut self) -> Trace {
        tokio::time::timeout(Duration::from_secs(2), self.traces.recv())
            .await
            .expect("TLS fixture did not finish its handshake")
            .expect("TLS fixture exited")
    }
}

impl Drop for TlsFixture {
    fn drop(&mut self) {
        std::fs::remove_file(&self.ca_file).unwrap();
        std::fs::remove_dir(self.ca_file.parent().unwrap()).unwrap();
    }
}

trait Socket: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Socket for T {}

async fn connection(
    socket: TcpStream,
    proxy_mode: ProxyMode,
    proxy_tls: Option<Arc<rustls::ServerConfig>>,
    target_mode: TargetMode,
    target_tls: Arc<rustls::ServerConfig>,
) -> Trace {
    let mut trace = Trace::default();
    let mut socket: Box<dyn Socket> = if let Some(config) = proxy_tls {
        match TlsAcceptor::from(config).accept(socket).await {
            Ok(socket) => {
                trace.proxy_tls_accepted = true;
                Box::new(socket)
            }
            Err(_) => {
                trace.proxy_tls_rejected = true;
                return trace;
            }
        }
    } else {
        Box::new(socket)
    };
    if !matches!(proxy_mode, ProxyMode::Direct) {
        trace.connect_request = Some(read_headers(&mut socket).await);
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
    }
    let hello = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), socket)
        .await
        .unwrap();
    trace.target_sni = hello.client_hello().server_name().map(str::to_owned);
    if matches!(target_mode, TargetMode::Eof) {
        // Close only after observing a real target ClientHello, after CONNECT.
        return trace;
    }
    match hello.into_stream(target_tls).await {
        Ok(mut socket) => {
            trace.target_tls_accepted = true;
            trace.target_request = Some(read_headers(&mut socket).await);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ntls-ok",
                )
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        }
        Err(_) => trace.target_tls_rejected = true,
    }
    trace
}

async fn read_headers(socket: &mut (impl AsyncRead + Unpin)) -> String {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        request.push(socket.read_u8().await.unwrap());
        assert!(request.len() < 16 * 1024);
    }
    String::from_utf8(request).unwrap()
}

struct Authority {
    params: CertificateParams,
    key: KeyPair,
    certificate: rcgen::Certificate,
}

impl Authority {
    fn new() -> Self {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let key = KeyPair::generate().unwrap();
        let certificate = params.self_signed(&key).unwrap();
        Self {
            params,
            key,
            certificate,
        }
    }

    fn server_config(&self, name: &str) -> Arc<rustls::ServerConfig> {
        let mut params = CertificateParams::new(vec![name.to_owned()]).unwrap();
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let key = KeyPair::generate().unwrap();
        let issuer = rcgen::Issuer::from_params(&self.params, &self.key);
        let certificate = params.signed_by(&key, &issuer).unwrap();
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(provider))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![certificate.der().clone()],
                    PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
                )
                .unwrap(),
        )
    }
}
