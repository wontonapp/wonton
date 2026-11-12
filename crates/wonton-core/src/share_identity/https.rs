use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use reqwest::{Client, StatusCode};
use url::Url;
use wonton_protocol::ShareIdentityFailure;

use super::{
    IdentityDns, IdentityFuture, MAX_ADDRESSES, MAX_REDIRECTS, MAX_RESPONSE_BYTES, MatrixServerName,
};

/// Trusted paired lookup, wired to the existing authenticated FederationForwarder. A match is
/// terminal: failures never fall through to public DNS. The forwarder passes TLS through iroh;
/// this module still verifies the certificate chain and the original Matrix server name.
/// Keep the returned forwarder alive while the request runs; revocation belongs to its lookup.
pub type PairedLookup = Arc<
    dyn Fn(&MatrixServerName) -> Option<Arc<wonton_tunnel::federation::FederationForwarder>>
        + Send
        + Sync,
>;

#[derive(Clone, Debug)]
pub(super) struct Destination {
    pub authority: MatrixServerName,
    pub target: String,
    pub port: u16,
}

impl Destination {
    pub(super) fn direct(name: MatrixServerName, default_port: u16) -> Self {
        Self {
            target: name.host.clone(),
            port: name.explicit_port.unwrap_or(default_port),
            authority: name,
        }
    }

    pub(super) fn url(&self, path: &str) -> Result<Url, ShareIdentityFailure> {
        Url::parse(&format!(
            "https://{}:{}{path}",
            self.authority.host, self.port
        ))
        .map_err(|_| ShareIdentityFailure::DiscoveryFailed)
    }
}

/// No Debug: even a Matrix-mandated userinfo query may contain the opaque access token.
pub(super) struct ExchangeRequest {
    pub url: Url,
    pub host_header: Option<String>,
    pub addresses: Vec<SocketAddr>,
    pub paired: Option<Arc<wonton_tunnel::federation::FederationForwarder>>,
}

pub(super) struct HttpResponse {
    pub status: StatusCode,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

pub(super) trait Exchange: Send + Sync {
    fn get(
        &self,
        request: ExchangeRequest,
    ) -> IdentityFuture<'_, Result<HttpResponse, ShareIdentityFailure>>;
}

pub(super) struct HttpsExchange {
    pub roots: Vec<reqwest::Certificate>,
}

impl Exchange for HttpsExchange {
    fn get(
        &self,
        request: ExchangeRequest,
    ) -> IdentityFuture<'_, Result<HttpResponse, ShareIdentityFailure>> {
        Box::pin(async move {
            let fail = ShareIdentityFailure::UserInfoFailed;
            let mut builder = Client::builder()
                .no_proxy()
                .https_only(true)
                .tls_backend_rustls()
                .redirect(reqwest::redirect::Policy::none())
                .referer(false)
                .retry(reqwest::retry::never())
                .timeout(super::TOTAL_TIMEOUT)
                .pool_max_idle_per_host(0);
            for root in &self.roots {
                builder = builder.add_root_certificate(root.clone());
            }
            if let Some(forwarder) = request.paired.as_ref() {
                // Explicit local CONNECT proxy only; never environment/system proxy configuration.
                let proxy = reqwest::Proxy::https(format!("http://{}", forwarder.listen_addr()))
                    .map_err(|_| fail)?;
                builder = builder.proxy(proxy);
            } else {
                builder = builder
                    .resolve_to_addrs(request.url.host_str().ok_or(fail)?, &request.addresses);
            }
            let client = builder.build().map_err(|_| fail)?;
            // Do not propagate reqwest errors: their Debug/Display may include the secret query.
            let mut outgoing = client.get(request.url);
            if let Some(host) = request.host_header {
                outgoing = outgoing.header(reqwest::header::HOST, host);
            }
            let mut response = outgoing
                .send()
                .await
                .map_err(|_| ShareIdentityFailure::DiscoveryFailed)?;
            let status = response.status();
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .map(|h| h.to_str().map(str::to_owned))
                .transpose()
                .map_err(|_| fail)?;
            if response
                .content_length()
                .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
            {
                return Err(fail);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| fail)? {
                append_bounded(&mut body, &chunk)?;
            }
            Ok(HttpResponse {
                status,
                location,
                body,
            })
        })
    }
}

pub(super) fn append_bounded(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), ShareIdentityFailure> {
    if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
        return Err(ShareIdentityFailure::UserInfoFailed);
    }
    body.extend_from_slice(chunk);
    Ok(())
}

pub(super) async fn resolve_public(
    dns: &dyn IdentityDns,
    target: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, ShareIdentityFailure> {
    let name = MatrixServerName::parse(target)?;
    let addresses = if let Some(ip) = name.ip() {
        vec![ip]
    } else {
        dns.addresses(&name.host).await?
    };
    public_addresses(&addresses)?;
    Ok(addresses
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect())
}

pub(super) fn public_addresses(addresses: &[IpAddr]) -> Result<(), ShareIdentityFailure> {
    if addresses.is_empty()
        || addresses.len() > MAX_ADDRESSES
        || addresses.iter().any(|ip| !globally_reachable(*ip))
    {
        return Err(ShareIdentityFailure::DiscoveryFailed);
    }
    Ok(())
}

fn globally_reachable(ip: IpAddr) -> bool {
    if crate::net::special::classify(ip).is_some() {
        return false;
    }
    let IpAddr::V6(v6) = ip else { return true };
    if v6.to_ipv4().is_some() {
        return true;
    }
    let s = v6.segments();
    // Preserve the shared classifier's globally reachable NAT64 treatment. Other IPv6
    // destinations must be allocated global unicast. In particular fec0::/10 is deprecated
    // site-local, not publicly routable, although the shared classifier currently permits it.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return true;
    }
    if s[0] & 0xe000 != 0x2000 {
        return false;
    }
    // IANA non-global ranges not covered by the existing classifier: Teredo, benchmarking,
    // ORCHID/ORCHIDv2 and the additional documentation allocation.
    !(s[0] == 0x2001 && (s[1] == 0 || s[1] == 2 || s[1] & 0xfff0 == 0x10 || s[1] & 0xfff0 == 0x20)
        || s[0] == 0x3fff && s[1] & 0xf000 == 0)
}

/// Public, bounded discovery fetch. Every hop gets a fresh validated, pinned DNS answer.
/// This helper never accepts credentials or a query; userinfo uses a non-redirecting path.
pub(super) async fn discovery_get(
    dns: &dyn IdentityDns,
    http: &dyn Exchange,
    mut url: Url,
) -> Result<HttpResponse, ShareIdentityFailure> {
    for redirects in 0..=MAX_REDIRECTS {
        validate_discovery_url(&url)?;
        let addresses = resolve_public(
            dns,
            url.host_str()
                .ok_or(ShareIdentityFailure::DiscoveryFailed)?,
            url.port_or_known_default().unwrap_or(443),
        )
        .await?;
        let response = http
            .get(ExchangeRequest {
                url: url.clone(),
                host_header: None,
                addresses,
                paired: None,
            })
            .await
            .map_err(|_| ShareIdentityFailure::DiscoveryFailed)?;
        if !response.status.is_redirection() {
            return Ok(response);
        }
        if redirects == MAX_REDIRECTS {
            break;
        }
        let location = response
            .location
            .ok_or(ShareIdentityFailure::DiscoveryFailed)?;
        url = url
            .join(&location)
            .map_err(|_| ShareIdentityFailure::DiscoveryFailed)?;
    }
    Err(ShareIdentityFailure::DiscoveryFailed)
}

fn validate_discovery_url(url: &Url) -> Result<(), ShareIdentityFailure> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 2048
    {
        return Err(ShareIdentityFailure::DiscoveryFailed);
    }
    Ok(())
}
