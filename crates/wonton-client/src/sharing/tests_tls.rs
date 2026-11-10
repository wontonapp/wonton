use super::discovery;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

async fn fixture(
    status: u16,
    body: String,
) -> (String, reqwest::Certificate, tokio::task::JoinHandle<()>) {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = params.self_signed(&ca_key).unwrap();
    let root = discovery::trusted_ca_bundle(ca.pem().as_bytes())
        .unwrap()
        .remove(0);
    let signing_key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["source.test".into()])
        .unwrap()
        .signed_by(&signing_key, &rcgen::Issuer::from_params(&params, &ca_key))
        .unwrap();
    // 客户端 dev 依赖开的是 ring，工作区其他 crate 开的是 aws-lc-rs：进程里有两个 provider 时要显式选一个。
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let body = body.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(stream).await else {
                    return;
                };
                let mut request = [0; 4096];
                if stream.read(&mut request).await.is_err() {
                    return;
                }
                let response = format!(
                    "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (format!("source.test:{}", addr.port()), root, task)
}

fn client(server: &str, root: Option<reqwest::Certificate>) -> reqwest::Client {
    let (host, port) = server.rsplit_once(':').unwrap();
    let mut builder =
        discovery::client_builder().resolve(host, format!("127.0.0.1:{port}").parse().unwrap());
    if let Some(root) = root {
        builder = builder.add_root_certificate(root);
    }
    builder.build().unwrap()
}

async fn attempt(
    client: &reqwest::Client,
    server: &str,
    requests: &AtomicUsize,
) -> Result<Option<(discovery::Discovery, ())>, wonton_protocol::ShareProblem> {
    discovery::discover_and_token(client, server, async {
        requests.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn only_validated_tls_discovery_polls_the_fresh_token_request() {
    let requests = AtomicUsize::new(0);
    let valid = serde_json::to_string(&wonton_protocol::WontonDiscovery {
        endpoint_id: iroh::SecretKey::generate().public().to_string(),
        relay_urls: vec![],
    })
    .unwrap();
    let (server, root, task) = fixture(200, valid).await;
    assert!(
        attempt(&client(&server, None), &server, &requests)
            .await
            .is_err()
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert!(
        attempt(&client(&server, Some(root)), &server, &requests)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    task.abort();
    let (server, root, task) = fixture(
        200,
        serde_json::to_string(&wonton_protocol::WontonDiscovery {
            endpoint_id: iroh::SecretKey::generate().public().to_string(),
            relay_urls: vec![],
        })
        .unwrap(),
    )
    .await;
    let wrong_name = server.replace("source.test", "wrong.test");
    assert!(
        attempt(&client(&wrong_name, Some(root)), &wrong_name, &requests)
            .await
            .is_err()
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    task.abort();
    for (status, body) in [
        (404, String::new()),
        (200, "{}".into()),
        (302, String::new()),
    ] {
        let (server, root, task) = fixture(status, body).await;
        let result = attempt(&client(&server, Some(root)), &server, &requests).await;
        if status == 404 {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err());
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn discovery_accepts_exactly_eight_kib_and_rejects_one_more_byte() {
    let record = serde_json::to_string(&wonton_protocol::WontonDiscovery {
        endpoint_id: iroh::SecretKey::generate().public().to_string(),
        relay_urls: vec![],
    })
    .unwrap();
    let body = format!("{record}{}", " ".repeat(8192 - record.len()));
    assert_eq!(body.len(), 8192);
    let (server, root, task) = fixture(200, body.clone()).await;
    assert!(
        discovery::discover(&client(&server, Some(root)), &server)
            .await
            .unwrap()
            .is_some()
    );
    task.abort();
    let body = format!("{body} ");
    assert_eq!(body.len(), 8193);
    let (server, root, task) = fixture(200, body).await;
    assert!(
        discovery::discover(&client(&server, Some(root)), &server)
            .await
            .is_err()
    );
    task.abort();
}

#[test]
fn explicit_ca_rejects_empty_malformed_leaf_and_expired_certificates() {
    for pem in [
        b"".as_slice(),
        b"not a certificate".as_slice(),
        b"-----BEGIN CERTIFICATE-----\ninvalid\n-----END CERTIFICATE-----".as_slice(),
    ] {
        assert!(discovery::trusted_ca_bundle(pem).is_err());
    }
    let rcgen::CertifiedKey { cert, .. } =
        rcgen::generate_simple_self_signed(vec!["source.test".into()]).unwrap();
    assert!(discovery::trusted_ca_bundle(cert.pem().as_bytes()).is_err());
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.not_before = rcgen::date_time_ymd(1999, 1, 1);
    params.not_after = rcgen::date_time_ymd(2000, 1, 1);
    let certificate = params
        .self_signed(&rcgen::KeyPair::generate().unwrap())
        .unwrap();
    assert!(discovery::trusted_ca_bundle(certificate.pem().as_bytes()).is_err());
}
