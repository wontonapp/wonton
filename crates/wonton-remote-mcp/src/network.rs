mod address;

use std::error::Error;
use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use reqwest::dns::{Name, Resolve, Resolving};
use reqwest::header::HeaderMap;
use reqwest::{Body, Client, Method, Response};
use url::{Host, Url};

use crate::config::{Config, validate_proxy};

use address::{validate_addresses, validate_url};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    BlockedAddress,
    Unreachable,
    ProxyUnavailable,
    InvalidInput,
}

impl NetworkError {
    pub fn outcome(&self) -> &'static str {
        match self {
            Self::BlockedAddress => "blocked_address",
            Self::Unreachable => "unreachable",
            Self::ProxyUnavailable => "proxy_unavailable",
            Self::InvalidInput => "invalid_input",
        }
    }

    pub fn detail(&self) -> &'static str {
        match self {
            Self::BlockedAddress => "The remote address is not permitted.",
            Self::Unreachable => "The remote server could not be reached.",
            Self::ProxyUnavailable => "The configured egress proxy is unavailable.",
            Self::InvalidInput => "The remote connection configuration is invalid.",
        }
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl Error for NetworkError {}

#[derive(Clone)]
pub(crate) struct Network {
    config: Arc<Config>,
}

impl Network {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    pub(crate) async fn send(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        body: Option<Body>,
    ) -> Result<Response, NetworkError> {
        // This deadline ends at response headers, not at the end of an SSE stream.
        tokio::time::timeout(
            self.config.connect_timeout,
            self.send_inner(method, url, headers, body),
        )
        .await
        .map_err(|_| NetworkError::Unreachable)?
    }

    async fn send_inner(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        body: Option<Body>,
    ) -> Result<Response, NetworkError> {
        let host = validate_url(url, self.config.address_policy)?;
        let addresses = match url.host() {
            Some(Host::Ipv4(address)) => vec![IpAddr::V4(address)],
            Some(Host::Ipv6(address)) => vec![IpAddr::V6(address)],
            Some(Host::Domain(_)) => self
                .config
                .resolver
                .resolve(host)
                .await
                .map_err(|_| NetworkError::Unreachable)?,
            None => return Err(NetworkError::InvalidInput),
        };
        validate_addresses(&addresses, &self.config)?;
        let client = self.client(host, &addresses)?;
        let mut request = client.request(method, url.clone()).headers(headers);
        if let Some(body) = body {
            request = request.body(body);
        }
        request
            .send()
            .await
            .map_err(|error| classify_request_error(&error, self.config.proxy.is_some()))
    }

    fn client(&self, host: &str, addresses: &[IpAddr]) -> Result<Client, NetworkError> {
        // A client is not shared across sends: even an active HTTP/2 connection
        // must not let a later request bypass fresh DNS validation.
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .pool_max_idle_per_host(0)
            .connect_timeout(self.config.connect_timeout);
        if let Some(proxy) = &self.config.proxy {
            validate_proxy(proxy)?;
            // D5: the proxy resolves the hostname again after our policy check.
            let proxy =
                reqwest::Proxy::all(proxy.as_str()).map_err(|_| NetworkError::InvalidInput)?;
            builder = builder.proxy(proxy);
        } else {
            let addresses: Vec<_> = addresses
                .iter()
                .map(|address| SocketAddr::new(*address, 0))
                .collect();
            // Keep the original URL for Host, certificate verification and SNI.
            builder = builder
                .dns_resolver(RejectDns)
                .resolve_to_addrs(host, &addresses);
        }
        builder.build().map_err(|_| NetworkError::InvalidInput)
    }
}

fn classify_request_error(error: &reqwest::Error, proxied: bool) -> NetworkError {
    let mut source = error.source();
    while let Some(cause) = source {
        // rustls reports a rejected certificate/handshake as InvalidData, rather
        // than a failure to establish the proxy's TCP connection or CONNECT tunnel.
        if cause
            .downcast_ref::<io::Error>()
            .is_some_and(|error| error.kind() == io::ErrorKind::InvalidData)
        {
            return NetworkError::Unreachable;
        }
        source = cause.source();
    }
    if proxied && error.is_connect() {
        NetworkError::ProxyUnavailable
    } else {
        NetworkError::Unreachable
    }
}

struct RejectDns;

impl Resolve for RejectDns {
    fn resolve(&self, _name: Name) -> Resolving {
        Box::pin(async {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unvalidated DNS request").into())
        })
    }
}
