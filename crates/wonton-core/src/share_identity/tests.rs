use std::{
    collections::VecDeque,
    net::IpAddr,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use reqwest::StatusCode;
use wonton_protocol::SecretString;

use super::https::{HttpResponse, append_bounded, discovery_get, public_addresses};
use super::*;

type DnsAnswer = Result<Option<Vec<SrvRecord>>, ShareIdentityFailure>;
struct FakeDns {
    ips: Mutex<VecDeque<Vec<IpAddr>>>,
    srv: Mutex<VecDeque<DnsAnswer>>,
    queries: Mutex<Vec<String>>,
}
impl FakeDns {
    fn public() -> Arc<Self> {
        Arc::new(Self {
            ips: Mutex::new(VecDeque::new()),
            srv: Mutex::new(VecDeque::new()),
            queries: Mutex::new(Vec::new()),
        })
    }
}
impl IdentityDns for FakeDns {
    fn addresses<'a>(
        &'a self,
        host: &'a str,
    ) -> IdentityFuture<'a, Result<Vec<IpAddr>, ShareIdentityFailure>> {
        Box::pin(async move {
            self.queries.lock().unwrap().push(host.into());
            Ok(self
                .ips
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| vec!["8.8.8.8".parse().unwrap()]))
        })
    }
    fn srv<'a>(&'a self, query: &'a str) -> IdentityFuture<'a, DnsAnswer> {
        Box::pin(async move {
            self.queries.lock().unwrap().push(query.into());
            self.srv.lock().unwrap().pop_front().unwrap_or(Ok(None))
        })
    }
}
struct FakeHttp {
    responses: Mutex<VecDeque<Result<HttpResponse, ShareIdentityFailure>>>,
    hits: AtomicUsize,
    requests: Mutex<Vec<(String, Vec<std::net::SocketAddr>)>>,
}
impl FakeHttp {
    fn new(responses: Vec<HttpResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            hits: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        })
    }
}
impl Exchange for FakeHttp {
    fn get(
        &self,
        request: ExchangeRequest,
    ) -> IdentityFuture<'_, Result<HttpResponse, ShareIdentityFailure>> {
        Box::pin(async move {
            self.hits.fetch_add(1, Ordering::SeqCst);
            // Test recorder deliberately retains only the public path, not token-bearing URL.
            self.requests.lock().unwrap().push((
                format!(
                    "{}:{}{}",
                    request.url.host_str().unwrap(),
                    request.url.port_or_known_default().unwrap(),
                    request.url.path()
                ),
                request.addresses,
            ));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected HTTP exchange")
        })
    }
}
#[derive(Default)]
struct Clock(AtomicU64);
impl DiscoveryClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}
fn verifier(dns: Arc<FakeDns>, http: Arc<FakeHttp>, clock: Arc<Clock>) -> IdentityVerifier {
    let mut verifier = IdentityVerifier::with_dns(dns, Arc::new(|_| None), clock);
    verifier.http = http;
    verifier
}
fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status: StatusCode::from_u16(status).unwrap(),
        location: None,
        body: body.as_bytes().to_vec(),
    }
}
fn token(server: &str) -> MatrixOpenIdToken {
    MatrixOpenIdToken {
        access_token: SecretString::new("opaque-secret-for-one-call"),
        matrix_server_name: server.into(),
        expires_in: 3600,
    }
}
fn srv(target: &str, port: u16) -> SrvRecord {
    SrvRecord {
        priority: 0,
        weight: 1,
        port,
        target: target.into(),
    }
}

#[test]
fn exact_identity_not_suffix_or_delegate() {
    let identity = MatrixServerName::parse("HOME.EXAMPLE:08448").unwrap();
    for subject in ["@bob:home.example", "@bob:HOME.EXAMPLE:8448"] {
        assert_eq!(checked_subject(subject, &identity).unwrap(), subject);
    }
    for subject in [
        "@bob:evilhome.example",
        "@bob:home.example.evil",
        "@bob:delegated.example",
        "@bob:home.example:443",
    ] {
        assert_eq!(
            checked_subject(subject, &identity),
            Err(ShareIdentityFailure::ServerNameMismatch)
        );
    }
    assert_eq!(
        checked_subject("not-a-user-id", &identity),
        Err(ShareIdentityFailure::UserInfoFailed)
    );
}

#[test]
fn server_names_normalize_idna_ipv6_and_ports_without_url_syntax() {
    let idna = MatrixServerName::parse("BÜCHER.example").unwrap();
    assert_eq!(idna.authority(), "xn--bcher-kva.example");
    assert!(idna.same_identity(&MatrixServerName::parse("xn--bcher-kva.example:8448").unwrap()));
    let ipv6 = MatrixServerName::parse("[2001:4860:0000::8888]:08448").unwrap();
    assert_eq!(ipv6.authority(), "[2001:4860::8888]:8448");
    assert!(ipv6.same_identity(&MatrixServerName::parse("[2001:4860::8888]").unwrap()));
    for name in [
        "",
        "https://home.example",
        "user@home.example",
        "home.example/path",
        "home.example?x",
        "home.example#x",
        "home.example:0",
        "home.example:65536",
        "home.example:",
        "home.example:+443",
        "home.example:000443",
        "home.example:443:8448",
        "[::1",
        "2001:4860::1",
        "home%2eexample",
        "home.example\\evil",
        ".example",
        "home..example",
        "-bad.example",
        "home.example ",
    ] {
        assert!(MatrixServerName::parse(name).is_err(), "{name}");
    }
}

#[test]
fn every_nonpublic_address_family_and_mixed_answers_are_rejected() {
    for text in [
        "0.0.0.0",
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "100.64.0.1",
        "169.254.1.1",
        "192.0.2.1",
        "224.0.0.1",
        "240.0.0.1",
        "::",
        "::1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "2001:db8::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "::127.0.0.1",
        "64:ff9b::7f00:1",
        "fec0::1",
        "100::1",
        "2001::1",
        "2001:2::1",
        "2001:10::1",
        "2001:20::1",
        "3fff::1",
        "4000::1",
    ] {
        let ip = text.parse().unwrap();
        assert!(public_addresses(&[ip]).is_err(), "{text}");
        assert!(
            public_addresses(&["8.8.8.8".parse().unwrap(), ip]).is_err(),
            "mixed {text}"
        );
    }
    assert!(public_addresses(&[]).is_err());
    assert!(public_addresses(&vec!["8.8.8.8".parse().unwrap(); MAX_ADDRESSES]).is_ok());
    assert!(public_addresses(&vec!["8.8.8.8".parse().unwrap(); MAX_ADDRESSES + 1]).is_err());
    for ip in [
        "8.8.8.8",
        "2001:4860:4860::8888",
        "::ffff:8.8.8.8",
        "64:ff9b::808:808",
    ] {
        assert!(public_addresses(&[ip.parse().unwrap()]).is_ok(), "{ip}");
    }
}

#[test]
fn response_limit_accepts_exact_threshold_and_rejects_one_extra_byte() {
    let mut body = Vec::new();
    append_bounded(&mut body, &vec![b'x'; MAX_RESPONSE_BYTES - 1]).unwrap();
    assert_eq!(body.len(), MAX_RESPONSE_BYTES - 1);
    append_bounded(&mut body, b"x").unwrap();
    assert_eq!(body.len(), MAX_RESPONSE_BYTES);
    assert!(append_bounded(&mut body, b"x").is_err());
    assert_eq!(body.len(), MAX_RESPONSE_BYTES);
}

#[test]
fn cache_expiry_and_capacity_have_exact_boundaries() {
    let cache = Cache::new();
    cache.insert("server".into(), 7, Duration::ZERO);
    assert_eq!(
        cache.get("server", DISCOVERY_TTL - Duration::from_nanos(1)),
        Some(7)
    );
    assert_eq!(cache.get("server", DISCOVERY_TTL), None);
    for n in 0..CACHE_CAPACITY {
        cache.insert(n.to_string(), n, Duration::from_secs(n as u64));
    }
    assert_eq!(cache.len(), CACHE_CAPACITY);
    cache.insert(
        "extra".into(),
        99,
        Duration::from_secs(CACHE_CAPACITY as u64),
    );
    assert_eq!(cache.len(), CACHE_CAPACITY);
    assert_eq!(
        cache.get("0", Duration::from_secs(CACHE_CAPACITY as u64)),
        None
    );
}

#[tokio::test]
async fn token_is_always_checked_with_userinfo_and_never_cached() {
    let http = FakeHttp::new(vec![
        response(200, r#"{"sub":"@bob:home.example"}"#),
        response(401, "secret-error"),
    ]);
    let verifier = verifier(FakeDns::public(), http.clone(), Arc::new(Clock::default()));
    let mut input = token("home.example:8448");
    assert_eq!(verifier.verify(&input).await.unwrap(), "@bob:home.example");
    input.expires_in = u64::MAX;
    assert_eq!(
        verifier.verify(&input).await,
        Err(ShareIdentityFailure::InvalidToken)
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 2);
    input.expires_in = 0;
    assert_eq!(
        verifier.verify(&input).await,
        Err(ShareIdentityFailure::InvalidToken)
    );
    input.expires_in = 1;
    input.access_token = SecretString::new("");
    assert_eq!(
        verifier.verify(&input).await,
        Err(ShareIdentityFailure::InvalidToken)
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn token_length_bound_is_exact_and_opaque_whitespace_is_not_rewritten() {
    let http = FakeHttp::new(vec![response(401, ""), response(401, "")]);
    let verifier = verifier(FakeDns::public(), http.clone(), Arc::new(Clock::default()));
    let mut input = token("home.example:8448");
    input.access_token = SecretString::new("x".repeat(MAX_TOKEN_BYTES));
    assert_eq!(
        verifier.verify(&input).await,
        Err(ShareIdentityFailure::InvalidToken)
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 1);
    input.access_token = SecretString::new("x".repeat(MAX_TOKEN_BYTES + 1));
    assert_eq!(
        verifier.verify(&input).await,
        Err(ShareIdentityFailure::InvalidToken)
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 1);
    input.access_token = SecretString::new(" ");
    assert_eq!(
        verifier.verify(&input).await,
        Err(ShareIdentityFailure::InvalidToken)
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn delegation_changes_destination_not_identity_and_cache_expires() {
    let dns = FakeDns::public();
    let http = FakeHttp::new(vec![
        response(200, r#"{"m.server":"delegated.example:443"}"#),
        response(200, r#"{"sub":"@bob:home.example"}"#),
        response(200, r#"{"sub":"@bob:delegated.example"}"#),
        response(200, r#"{"m.server":"replacement.example:443"}"#),
        response(200, r#"{"sub":"@bob:home.example"}"#),
    ]);
    let clock = Arc::new(Clock::default());
    let verifier = verifier(dns, http.clone(), clock.clone());
    assert!(verifier.verify(&token("home.example")).await.is_ok());
    clock.0.store(3599, Ordering::SeqCst);
    assert_eq!(
        verifier.verify(&token("home.example")).await,
        Err(ShareIdentityFailure::ServerNameMismatch)
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 3);
    clock.0.store(3600, Ordering::SeqCst);
    assert!(verifier.verify(&token("home.example")).await.is_ok());
    let requests = http.requests.lock().unwrap();
    assert_eq!(
        requests[1].0,
        "delegated.example:443/_matrix/federation/v1/openid/userinfo"
    );
    assert_eq!(
        requests[4].0,
        "replacement.example:443/_matrix/federation/v1/openid/userinfo"
    );
}

#[tokio::test]
async fn matrix_srv_order_and_8448_fallback_are_spec_scoped() {
    let dns = FakeDns::public();
    dns.srv
        .lock()
        .unwrap()
        .extend([Ok(None), Ok(Some(vec![srv("target.example", 9448)]))]);
    let http = FakeHttp::new(vec![
        response(404, ""),
        response(200, r#"{"sub":"@bob:home.example"}"#),
    ]);
    let first = verifier(dns.clone(), http.clone(), Arc::new(Clock::default()));
    assert!(first.verify(&token("home.example")).await.is_ok());
    assert_eq!(
        &dns.queries.lock().unwrap()[1..3],
        &["_matrix-fed._tcp.home.example", "_matrix._tcp.home.example"]
    );
    {
        let requests = http.requests.lock().unwrap();
        assert_eq!(
            requests[1].0,
            "home.example:9448/_matrix/federation/v1/openid/userinfo"
        );
        assert_eq!(requests[1].1[0].port(), 9448);
    }
    let dns = FakeDns::public();
    let http = FakeHttp::new(vec![
        response(404, ""),
        response(200, r#"{"sub":"@bob:other.example"}"#),
    ]);
    let verifier = verifier(dns, http.clone(), Arc::new(Clock::default()));
    assert!(verifier.verify(&token("other.example")).await.is_ok());
    let second_request = http.requests.lock().unwrap()[1].0.clone();
    assert!(second_request.starts_with("other.example:8448/"));
}

#[tokio::test]
async fn srv_transient_failure_is_not_a_fallback_signal() {
    let dns = FakeDns::public();
    dns.srv
        .lock()
        .unwrap()
        .push_back(Err(ShareIdentityFailure::DiscoveryFailed));
    let http = FakeHttp::new(vec![response(404, "")]);
    let verifier = verifier(dns.clone(), http.clone(), Arc::new(Clock::default()));
    assert_eq!(
        verifier.verify(&token("home.example")).await,
        Err(ShareIdentityFailure::DiscoveryFailed)
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 1);
    assert_eq!(dns.queries.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn redirect_rebinding_and_downgrade_stop_before_second_request() {
    for location in [
        "https://home.example/next",
        "http://other.example/next",
        "https://user@other.example/next",
    ] {
        let dns = FakeDns::public();
        dns.ips.lock().unwrap().extend([
            vec!["8.8.8.8".parse().unwrap()],
            vec!["127.0.0.1".parse().unwrap()],
        ]);
        let mut redirect = response(302, "");
        redirect.location = Some(location.into());
        let http = FakeHttp::new(vec![redirect]);
        let result = discovery_get(
            dns.as_ref(),
            http.as_ref(),
            url::Url::parse("https://home.example/start").unwrap(),
        )
        .await;
        assert!(matches!(result, Err(ShareIdentityFailure::DiscoveryFailed)));
        assert_eq!(http.hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            http.requests.lock().unwrap()[0].1[0].ip(),
            "8.8.8.8".parse::<IpAddr>().unwrap()
        );
    }
}

#[tokio::test]
async fn userinfo_redirects_and_untrusted_error_bodies_never_escape() {
    for (status, body, expected) in [
        (
            302,
            "opaque-secret-for-one-call",
            ShareIdentityFailure::UserInfoFailed,
        ),
        (
            500,
            "opaque-secret-for-one-call",
            ShareIdentityFailure::UserInfoFailed,
        ),
        (
            403,
            "opaque-secret-for-one-call",
            ShareIdentityFailure::InvalidToken,
        ),
        (
            200,
            r#"{"sub":"@bob:evil.example"}"#,
            ShareIdentityFailure::ServerNameMismatch,
        ),
        (
            200,
            "not-json opaque-secret-for-one-call",
            ShareIdentityFailure::UserInfoFailed,
        ),
    ] {
        let http = FakeHttp::new(vec![response(status, body)]);
        let verifier = verifier(FakeDns::public(), http.clone(), Arc::new(Clock::default()));
        let input = token("home.example:8448");
        let failure = verifier.verify(&input).await.unwrap_err();
        assert_eq!(failure, expected);
        assert!(!format!("{failure:?} {input:?}").contains(input.access_token.expose()));
        assert_eq!(http.hits.load(Ordering::SeqCst), 1);
    }
}

struct DelayedHttp(Duration);
impl Exchange for DelayedHttp {
    fn get(
        &self,
        _: ExchangeRequest,
    ) -> IdentityFuture<'_, Result<HttpResponse, ShareIdentityFailure>> {
        Box::pin(async move {
            tokio::time::sleep(self.0).await;
            Ok(response(200, r#"{"sub":"@bob:home.example"}"#))
        })
    }
}

#[tokio::test(start_paused = true)]
async fn total_timeout_has_an_exact_deadline() {
    let mut instance = verifier(
        FakeDns::public(),
        FakeHttp::new(vec![]),
        Arc::new(Clock::default()),
    );
    instance.http = Arc::new(DelayedHttp(TOTAL_TIMEOUT - Duration::from_secs(1)));
    let input = token("home.example:8448");
    let operation = instance.verify(&input);
    tokio::pin!(operation);
    assert!(futures_util::poll!(&mut operation).is_pending());
    tokio::time::advance(TOTAL_TIMEOUT - Duration::from_secs(1)).await;
    assert!(operation.await.is_ok());
    let mut instance = verifier(
        FakeDns::public(),
        FakeHttp::new(vec![]),
        Arc::new(Clock::default()),
    );
    instance.http = Arc::new(DelayedHttp(TOTAL_TIMEOUT + Duration::from_secs(1)));
    let operation = instance.verify(&input);
    tokio::pin!(operation);
    assert!(futures_util::poll!(&mut operation).is_pending());
    tokio::time::advance(TOTAL_TIMEOUT - Duration::from_nanos(1)).await;
    assert!(futures_util::poll!(&mut operation).is_pending());
    tokio::time::advance(Duration::from_nanos(1)).await;
    assert_eq!(operation.await, Err(ShareIdentityFailure::UserInfoFailed));
}

#[tokio::test]
async fn redirect_count_accepts_five_hops_not_six() {
    for (redirects, succeeds, hits) in [
        (MAX_REDIRECTS, true, MAX_REDIRECTS + 1),
        (MAX_REDIRECTS + 1, false, MAX_REDIRECTS + 1),
    ] {
        let mut responses = Vec::new();
        for n in 0..redirects {
            let mut redirect = response(302, "");
            redirect.location = Some(format!("/hop/{n}"));
            responses.push(redirect);
        }
        responses.push(response(200, "{}"));
        let http = FakeHttp::new(responses);
        let result = discovery_get(
            FakeDns::public().as_ref(),
            http.as_ref(),
            url::Url::parse("https://home.example/start").unwrap(),
        )
        .await;
        assert_eq!(result.is_ok(), succeeds);
        assert_eq!(http.hits.load(Ordering::SeqCst), hits);
    }
}

#[tokio::test]
async fn wonton_discovery_validates_endpoint_relays_and_caches_only_successes() {
    let endpoint = iroh::SecretKey::generate().public().to_string();
    let good =
        serde_json::json!({"endpoint_id": endpoint, "relay_urls": ["https://relay.example/"]})
            .to_string();
    let bad = serde_json::json!({"endpoint_id": endpoint, "relay_urls": ["http://relay.example/"]})
        .to_string();
    let http = FakeHttp::new(vec![
        response(404, ""),
        response(200, &bad),
        response(200, &good),
        response(404, ""),
    ]);
    let clock = Arc::new(Clock::default());
    let instance = verifier(FakeDns::public(), http.clone(), clock.clone());
    assert!(
        instance
            .discover_wonton("home.example")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        instance.discover_wonton("home.example").await,
        Err(ShareIdentityFailure::DiscoveryFailed)
    );
    assert_eq!(
        instance
            .discover_wonton("home.example")
            .await
            .unwrap()
            .unwrap()
            .endpoint_id,
        endpoint
    );
    clock.0.store(3599, Ordering::SeqCst);
    assert!(
        instance
            .discover_wonton("home.example")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 3);
    clock.0.store(3600, Ordering::SeqCst);
    assert!(
        instance
            .discover_wonton("home.example")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(http.hits.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn paired_route_skips_dns_and_nondefault_port_fails_closed() {
    let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .bind()
        .await
        .unwrap();
    let (_paired, rx) = tokio::sync::watch::channel(std::collections::BTreeSet::new());
    let forwarder = Arc::new(
        wonton_tunnel::federation::FederationForwarder::bind(
            endpoint.clone(),
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(|_| None),
            rx,
            Duration::from_secs(60),
            Duration::from_secs(1),
        )
        .await
        .unwrap(),
    );
    let dns = FakeDns::public();
    let http = FakeHttp::new(vec![response(200, r#"{"sub":"@bob:paired.example"}"#)]);
    let mut instance = verifier(dns.clone(), http.clone(), Arc::new(Clock::default()));
    instance.paired = Arc::new(move |_| Some(forwarder.clone()));
    assert_eq!(
        instance.verify(&token("paired.example")).await.unwrap(),
        "@bob:paired.example"
    );
    assert!(dns.queries.lock().unwrap().is_empty());
    assert!(http.requests.lock().unwrap()[0].1.is_empty());
    assert_eq!(
        instance.verify(&token("paired.example:443")).await,
        Err(ShareIdentityFailure::DiscoveryFailed)
    );
    assert!(dns.queries.lock().unwrap().is_empty());
    assert_eq!(http.hits.load(Ordering::SeqCst), 1);
    drop(instance);
    endpoint.close().await;
}

#[tokio::test]
async fn modern_srv_prevents_legacy_lookup_and_root_target_prevents_fallback() {
    for target in ["modern.example", ""] {
        let dns = FakeDns::public();
        dns.srv
            .lock()
            .unwrap()
            .push_back(Ok(Some(vec![srv(target, 9448)])));
        let http = FakeHttp::new(vec![
            response(404, ""),
            response(200, r#"{"sub":"@bob:home.example"}"#),
        ]);
        let instance = verifier(dns.clone(), http.clone(), Arc::new(Clock::default()));
        let result = instance.verify(&token("home.example")).await;
        assert_eq!(result.is_ok(), !target.is_empty());
        if target.is_empty() {
            assert_eq!(result, Err(ShareIdentityFailure::DiscoveryFailed));
        }
        assert!(
            !dns.queries
                .lock()
                .unwrap()
                .iter()
                .any(|q| q.starts_with("_matrix._tcp"))
        );
    }
}

#[test]
fn srv_record_limit_is_exact() {
    let exact = vec![srv("target.example", 8448); MAX_SRV_RECORDS];
    assert_eq!(exact.len(), MAX_SRV_RECORDS);
    assert_eq!(dns::order_srv(exact).unwrap().len(), MAX_SRV_RECORDS);
    let excessive = vec![srv("target.example", 8448); MAX_SRV_RECORDS + 1];
    assert_eq!(excessive.len(), MAX_SRV_RECORDS + 1);
    assert!(dns::order_srv(excessive).is_err());
}

#[tokio::test]
async fn direct_private_literal_is_rejected_without_http_or_dns() {
    let dns = FakeDns::public();
    let http = FakeHttp::new(vec![]);
    let instance = verifier(dns.clone(), http.clone(), Arc::new(Clock::default()));
    for server in ["127.0.0.1:8448", "[::ffff:127.0.0.1]:8448", "[fc00::1]"] {
        assert_eq!(
            instance.verify(&token(server)).await,
            Err(ShareIdentityFailure::DiscoveryFailed)
        );
    }
    assert!(dns.queries.lock().unwrap().is_empty());
    assert_eq!(http.hits.load(Ordering::SeqCst), 0);
}

#[test]
fn ipv6_allocations_and_embedded_addresses_have_exact_edges() {
    for ip in [
        "1fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "4000::",
        "fec0::",
        "feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "2001:2:ffff:ffff:ffff:ffff:ffff:ffff",
        "2001:1f:ffff:ffff:ffff:ffff:ffff:ffff",
        "2001:2f:ffff:ffff:ffff:ffff:ffff:ffff",
        "3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff",
        "::ffff:192.168.255.255",
        "64:ff9b::c0a8:ffff",
    ] {
        assert!(public_addresses(&[ip.parse().unwrap()]).is_err(), "{ip}");
    }
    for ip in [
        "2000::",
        "3fff:1000::",
        "3fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "2001:3::",
        "2001:30::",
        "::ffff:192.169.0.0",
        "64:ff9b::c0a9:0",
    ] {
        assert!(public_addresses(&[ip.parse().unwrap()]).is_ok(), "{ip}");
    }
}
