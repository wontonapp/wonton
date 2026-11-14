use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ipnet::IpNet;
use url::Url;

use crate::network::NetworkError;

pub type ResolveFuture = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;

pub trait Resolver: Send + Sync {
    fn resolve(&self, host: &str) -> ResolveFuture;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressPolicy {
    PublicOnly,
    /// Only explicit library callers may allow local test servers.
    AllowPrivate,
}

#[derive(Clone)]
pub struct Config {
    pub address_policy: AddressPolicy,
    pub resolver: Arc<dyn Resolver>,
    pub proxy: Option<Url>,
    pub connect_timeout: Duration,
    pub probe_timeout: Duration,
    pub blocked_networks: Vec<IpNet>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            address_policy: AddressPolicy::PublicOnly,
            resolver: Arc::new(SystemResolver),
            proxy: None,
            connect_timeout: Duration::from_secs(10),
            probe_timeout: Duration::from_secs(15),
            blocked_networks: Vec::new(),
        }
    }
}

impl Config {
    /// Production has no environment-controlled relaxation of address policy.
    pub fn production() -> Result<Self, NetworkError> {
        let proxy_value = match environment("HTTPS_PROXY")? {
            Some(value) => Some(value),
            None => environment("https_proxy")?,
        };
        let proxy = proxy_value
            .map(|value| {
                let url = Url::parse(&value).map_err(|_| NetworkError::InvalidInput)?;
                validate_proxy(&url)?;
                Ok::<_, NetworkError>(url)
            })
            .transpose()?;
        let blocked_networks = environment("WONTON_REMOTE_MCP_BLOCKED_NETWORKS")?
            .map(|value| {
                value
                    .split(',')
                    .map(|network| {
                        network
                            .trim()
                            .parse::<IpNet>()
                            .map_err(|_| NetworkError::InvalidInput)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            proxy,
            blocked_networks,
            ..Self::default()
        })
    }
}

pub(crate) fn validate_proxy(url: &Url) -> Result<(), NetworkError> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || url.port() == Some(0)
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(NetworkError::InvalidInput);
    }
    Ok(())
}

fn environment(name: &str) -> Result<Option<String>, NetworkError> {
    std::env::var(name).map(Some).or_else(|error| match error {
        std::env::VarError::NotPresent => Ok(None),
        std::env::VarError::NotUnicode(_) => Err(NetworkError::InvalidInput),
    })
}

struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve(&self, host: &str) -> ResolveFuture {
        let host = host.to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addresses.map(|address| address.ip()).collect())
        })
    }
}
