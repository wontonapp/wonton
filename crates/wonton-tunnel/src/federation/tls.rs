//! 出站两段 TLS（ADR-0112 §2/§4）：对本机 Tuwunel 当 TLS 服务端，证书按 CONNECT 的目标名现签
//! （Tuwunel 的信任文件里有本机 CA）；对对方当 TLS 客户端，根库只放钉住的叶，另单独比 SPKI 并校名字。
//!
//! 两段都是安全边界：只比指纹不校名字不算数（规范要求证书名对上），只靠根库也不够（ADR 实测：
//! 根库放签发它的 CA 或自签叶都能过，所以 SPKI 要单独比）。没有 pin 的旧记录按“待钉”只校名字放行
//! （§5b），窗口由调用方按记录决定。

use std::{
    fmt, io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use rustls::{
    ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
    client::WebPkiServerVerifier,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{
        CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::ParsedCertificate,
};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};
use tokio_rustls::TlsAcceptor;

use crate::client::TunnelStream;

/// 对方的一张钉证书：整张 DER 当根，SPKI 的 SHA-256 单独比。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedPeerCert {
    cert: CertificateDer<'static>,
    spki_sha256: [u8; 32],
}

impl PinnedPeerCert {
    /// 从对方配对时互换的叶证书建一条 pin。证书解析不了当场失败，不等到握手。
    pub fn from_der(cert: CertificateDer<'static>) -> Result<Self, Error> {
        let spki_sha256 = spki_sha256(&cert)?;
        Ok(Self { cert, spki_sha256 })
    }

    /// 用配对记录里存的 DER 和 SPKI 指纹建 pin。两者对不上（记录坏了）当场失败。
    pub fn from_parts(der: Vec<u8>, spki_sha256: [u8; 32]) -> Result<Self, Error> {
        let cert = CertificateDer::from(der);
        if self::spki_sha256(&cert)? != spki_sha256 {
            return Err(Error::General(
                "the stored SPKI fingerprint does not match the certificate".into(),
            ));
        }
        Ok(Self { cert, spki_sha256 })
    }

    /// 钉住的整张叶证书，进根库。
    pub fn cert(&self) -> &CertificateDer<'static> {
        &self.cert
    }

    /// 叶证书 SPKI 的 SHA-256。
    pub fn spki_sha256(&self) -> [u8; 32] {
        self.spki_sha256
    }
}

/// 一台对方服务器的校验严格度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerVerification {
    /// 钉住的叶（轮换期间新旧都在）：根库只放它们，SPKI 必须是其中一个，名字照校。
    Pinned(Vec<PinnedPeerCert>),
    /// 旧记录还没有 pin：只校名字，证书不在任何根库里也放行（ADR-0112 §5b）。
    Pending,
}

/// 按本机 Tuwunel 要连的服务器名出一张证书（本机 CA 签）。
pub type LocalCertificateIssuer = Arc<dyn Fn(&str) -> anyhow::Result<TlsAcceptor> + Send + Sync>;

/// 按对方服务器名给出钉住的叶；`None` = 旧记录待钉。
pub type PeerPinLookup = Arc<dyn Fn(&str) -> Option<PeerVerification> + Send + Sync>;

/// 出站两段握手要的本地材料。
pub struct ForwarderTls {
    /// 按 CONNECT 的目标名给本机 Tuwunel 出示证书（本机 CA 签）。
    pub local_certificates: LocalCertificateIssuer,
    /// 按对方服务器名给出钉证书；`None` = 待钉。
    pub peer_pin: PeerPinLookup,
}

impl fmt::Debug for ForwarderTls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForwarderTls").finish_non_exhaustive()
    }
}

/// 对对方当 TLS 客户端的配置。
pub(super) fn client_config(verification: &PeerVerification) -> Result<Arc<ClientConfig>, Error> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier: Arc<dyn ServerCertVerifier> = match verification {
        PeerVerification::Pinned(pins) => Arc::new(PinnedVerifier::new(pins, &provider)?),
        PeerVerification::Pending => Arc::new(PendingVerifier {
            algorithms: provider.signature_verification_algorithms,
        }),
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// 钉证书的校验：先让 `WebPkiServerVerifier` 按“只含钉叶的根库”验证链和名字，再单独比 SPKI。
struct PinnedVerifier {
    inner: Arc<WebPkiServerVerifier>,
    pins: Vec<[u8; 32]>,
}

impl PinnedVerifier {
    fn new(pins: &[PinnedPeerCert], provider: &CryptoProvider) -> Result<Self, Error> {
        let mut roots = RootCertStore::empty();
        for pin in pins {
            roots.add(pin.cert.clone()).map_err(|_| {
                Error::General("a pinned certificate cannot be a trust anchor".into())
            })?;
        }
        let inner = WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(provider.clone()),
        )
        .build()
        .map_err(|err| Error::General(err.to_string()))?;
        Ok(Self {
            inner,
            pins: pins.iter().map(PinnedPeerCert::spki_sha256).collect(),
        })
    }
}

impl fmt::Debug for PinnedVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinnedVerifier")
            .field("pins", &self.pins)
            .finish_non_exhaustive()
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let verified = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        if !self.pins.contains(&spki_sha256(end_entity)?) {
            return Err(Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(verified)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// 待钉的校验：链不看（对方还没有可比的 pin），但证书要能解析、在有效期里、名字对得上。
struct PendingVerifier {
    algorithms: WebPkiSupportedAlgorithms,
}

impl fmt::Debug for PendingVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingVerifier").finish_non_exhaustive()
    }
}

impl ServerCertVerifier for PendingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let (_, cert) = x509_parser::parse_x509_certificate(end_entity.as_ref())
            .map_err(|_| Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
        let validity = cert.validity();
        if !validity.is_valid() {
            let error = if validity.not_before.timestamp() > validity.not_after.timestamp() {
                rustls::CertificateError::BadEncoding
            } else if validity.not_before.timestamp() > unix_now() {
                rustls::CertificateError::NotValidYet
            } else {
                rustls::CertificateError::Expired
            };
            return Err(Error::InvalidCertificate(error));
        }
        let parsed = ParsedCertificate::try_from(end_entity)?;
        rustls::client::verify_server_name(&parsed, server_name)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn spki_sha256(cert: &CertificateDer<'_>) -> Result<[u8; 32], Error> {
    let (_, cert) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|_| Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
    Ok(Sha256::digest(cert.tbs_certificate.subject_pki.raw).into())
}

/// 把握手失败翻成给状态和日志看的短句。原因是给别人读的，不带我们自己路径。
pub(super) fn handshake_reason(err: &io::Error) -> String {
    use rustls::CertificateError;
    let Some(rustls_err) = err.get_ref().and_then(|err| err.downcast_ref::<Error>()) else {
        return format!("the TLS handshake failed: {err}");
    };
    let Error::InvalidCertificate(reason) = rustls_err else {
        return format!("the TLS handshake failed: {rustls_err}");
    };
    match reason {
        CertificateError::ApplicationVerificationFailure => {
            "the certificate does not match the pinned key".into()
        }
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
            "the certificate is not valid for the server name".into()
        }
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
            "the peer certificate expired".into()
        }
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
            "the peer certificate is not valid yet".into()
        }
        CertificateError::UnknownIssuer => "the certificate is not the pinned one".into(),
        other => format!("the peer certificate failed validation: {other}"),
    }
}

/// 隧道流读写的适配：TLS 层当普通 `AsyncRead`/`AsyncWrite` 用。
pub(super) struct TunnelIo(pub(super) TunnelStream);

impl TunnelIo {
    pub(super) fn abort(&mut self) {
        self.0.abort();
    }
}

impl AsyncRead for TunnelIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.0.recv), cx, buf)
    }
}

impl AsyncWrite for TunnelIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.0.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.0.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.0.send), cx)
    }
}

/// 先吐出已读到的字节（Tuwunel 常常把 ClientHello 和 CONNECT 一起发来），再读连接。
pub(super) struct PrefixedStream<S> {
    inner: S,
    prefix: Vec<u8>,
    at: usize,
}

impl<S> PrefixedStream<S> {
    pub(super) fn new(inner: S, prefix: Vec<u8>) -> Self {
        Self {
            inner,
            prefix,
            at: 0,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let left = self.prefix.len().saturating_sub(self.at);
        if left > 0 {
            let take = left.min(buf.remaining());
            buf.put_slice(&self.prefix[self.at..self.at + take]);
            self.at += take;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// TLS 服务端监听在 `tcp` 上，`rest` 是 CONNECT 头之后已经读到的字节（属于 ClientHello）。
pub(super) async fn accept_local(
    acceptor: &TlsAcceptor,
    tcp: TcpStream,
    rest: Vec<u8>,
) -> io::Result<tokio_rustls::server::TlsStream<PrefixedStream<TcpStream>>> {
    acceptor.accept(PrefixedStream::new(tcp, rest)).await
}
