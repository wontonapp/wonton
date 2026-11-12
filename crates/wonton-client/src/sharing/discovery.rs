use super::problem;
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use reqwest::{Client, StatusCode};
use std::time::Duration;
use url::Url;
use wonton_protocol::{ShareFailure, ShareProblem, WONTON_WELL_KNOWN_PATH, WontonDiscovery};

pub(super) struct Discovery {
    pub target: EndpointAddr,
    pub relays: Vec<RelayUrl>,
}

pub(super) async fn discover_and_token<T>(
    client: &Client,
    server: &str,
    token: impl Future<Output = Result<T, ShareProblem>>,
) -> Result<Option<(Discovery, T)>, ShareProblem> {
    let Some(discovery) = discover(client, server).await? else {
        return Ok(None);
    };
    Ok(Some((discovery, token.await?)))
}

pub(super) async fn http_client() -> Result<Client, ShareProblem> {
    if let Err(reason) = wonton_matrix::platform_tls::check() {
        tracing::warn!("cannot verify HTTPS certificates: {reason}");
        return Err(problem(ShareFailure::Unreachable, None));
    }
    let mut builder = client_builder();
    if let Some(path) = std::env::var_os("WONTON_SHARE_CA_FILE") {
        let pem = tokio::fs::read(path)
            .await
            .map_err(|_| problem(ShareFailure::Other, None))?;
        for certificate in trusted_ca_bundle(&pem)? {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|_| problem(ShareFailure::Unreachable, None))
}

pub(super) fn client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
}

pub(super) fn trusted_ca_bundle(pem: &[u8]) -> Result<Vec<reqwest::Certificate>, ShareProblem> {
    let fail = || problem(ShareFailure::Other, None);
    let mut certificates = Vec::new();
    for entry in x509_parser::pem::Pem::iter_from_buffer(pem) {
        let entry = entry.map_err(|_| fail())?;
        if entry.label != "CERTIFICATE" {
            return Err(fail());
        }
        let (remaining, certificate) =
            x509_parser::parse_x509_certificate(&entry.contents).map_err(|_| fail())?;
        if !remaining.is_empty()
            || !certificate.validity().is_valid()
            || !certificate
                .basic_constraints()
                .map_err(|_| fail())?
                .is_some_and(|constraints| constraints.value.ca)
            || certificate
                .key_usage()
                .map_err(|_| fail())?
                .is_some_and(|usage| !usage.value.key_cert_sign())
        {
            return Err(fail());
        }
        certificates.push(reqwest::Certificate::from_der(&entry.contents).map_err(|_| fail())?);
    }
    if certificates.is_empty() {
        return Err(fail());
    }
    Ok(certificates)
}

fn discovery_url(server: &str) -> Result<Url, ShareProblem> {
    let fail = || problem(ShareFailure::Other, None);
    if server.is_empty()
        || server.len() > 255
        || !server.is_ascii()
        || server
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(fail());
    }
    let mut url = Url::parse(&format!("https://{server}")).map_err(|_| fail())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
        || server.contains(['/', '\\', '%', '?', '#'])
    {
        return Err(fail());
    }
    if let Some(url::Host::Domain(host)) = url.host() {
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.is_empty()
            || host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(fail());
        }
    } else if url.host().is_none() {
        return Err(fail());
    }
    url.set_path(WONTON_WELL_KNOWN_PATH);
    Ok(url)
}

pub(super) async fn discover(
    client: &Client,
    server: &str,
) -> Result<Option<Discovery>, ShareProblem> {
    let url = discovery_url(server)?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| problem(ShareFailure::Unreachable, None))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(problem(
            ShareFailure::Unreachable,
            Some(response.status().as_u16()),
        ));
    }
    if response.content_length().is_some_and(|n| n > 8192) {
        return Err(problem(ShareFailure::Other, None));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| problem(ShareFailure::Unreachable, None))?
    {
        if chunk.len() > 8192 - bytes.len() {
            return Err(problem(ShareFailure::Other, None));
        }
        bytes.extend_from_slice(&chunk);
    }
    let record: WontonDiscovery =
        serde_json::from_slice(&bytes).map_err(|_| problem(ShareFailure::Other, None))?;
    validate(record).map(Some)
}

pub(super) fn validate(record: WontonDiscovery) -> Result<Discovery, ShareProblem> {
    let fail = || problem(ShareFailure::Other, None);
    let id: EndpointId = record.endpoint_id.parse().map_err(|_| fail())?;
    if id.to_string() != record.endpoint_id {
        return Err(fail());
    }
    let mut relays = Vec::new();
    let mut target = EndpointAddr::new(id);
    for text in record.relay_urls {
        let url = Url::parse(&text).map_err(|_| fail())?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
            || url.port() == Some(0)
        {
            return Err(fail());
        }
        let relay: RelayUrl = text.parse().map_err(|_| fail())?;
        target = target.with_relay_url(relay.clone());
        relays.push(relay);
    }
    Ok(Discovery { target, relays })
}
