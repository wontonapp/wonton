use std::{net::SocketAddr, sync::Arc};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    },
};

use super::https::{Exchange, ExchangeRequest, HttpsExchange};
use super::*;

struct LocalTls {
    address: SocketAddr,
    root: reqwest::Certificate,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for LocalTls {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl LocalTls {
    async fn start(name: &str, body: Vec<u8>, length_header: bool) -> Self {
        let cert = rcgen::generate_simple_self_signed(vec![name.to_owned()]).unwrap();
        let root = reqwest::Certificate::from_pem(cert.cert.pem().as_bytes()).unwrap();
        let chain = vec![CertificateDer::from(cert.cert.der().to_vec())];
        let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into();
        let provider = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider();
        let config = ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let (acceptor, body) = (acceptor.clone(), body.clone());
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let mut bytes = Vec::new();
                    let mut byte = [0; 1];
                    while bytes.len() < 4096 && !bytes.ends_with(b"\r\n\r\n") {
                        if tls.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        bytes.push(byte[0]);
                    }
                    let head = if length_header {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                    } else {
                        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".into()
                    };
                    if tls.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = tls.write_all(&body).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        Self {
            address,
            root,
            task,
        }
    }
    fn request(&self, host: &str) -> ExchangeRequest {
        ExchangeRequest {
            url: url::Url::parse(&format!("https://{host}:{}/discovery", self.address.port()))
                .unwrap(),
            // Only the private test-only transport invocation bypasses public-address policy.
            // The public verifier path always calls resolve_public before Exchange::get.
            host_header: None,
            addresses: vec![self.address],
            paired: None,
        }
    }
}

#[tokio::test]
async fn real_https_requires_trusted_chain_and_matching_name() {
    let server = LocalTls::start("valid.test", b"{}".to_vec(), true).await;
    let untrusted = HttpsExchange { roots: vec![] };
    assert!(untrusted.get(server.request("valid.test")).await.is_err());
    let trusted = HttpsExchange {
        roots: vec![server.root.clone()],
    };
    assert_eq!(
        trusted
            .get(server.request("valid.test"))
            .await
            .unwrap()
            .body,
        b"{}"
    );
    assert!(trusted.get(server.request("wrong.test")).await.is_err());
}

#[tokio::test]
async fn real_https_body_limits_apply_with_and_without_length_header() {
    for length_header in [false, true] {
        let exact =
            LocalTls::start("limit.test", vec![b'x'; MAX_RESPONSE_BYTES], length_header).await;
        let client = HttpsExchange {
            roots: vec![exact.root.clone()],
        };
        let body = client.get(exact.request("limit.test")).await.unwrap().body;
        assert_eq!(body.len(), MAX_RESPONSE_BYTES);
        let excessive = LocalTls::start(
            "limit.test",
            vec![b'x'; MAX_RESPONSE_BYTES + 1],
            length_header,
        )
        .await;
        let client = HttpsExchange {
            roots: vec![excessive.root.clone()],
        };
        assert!(client.get(excessive.request("limit.test")).await.is_err());
    }
}
