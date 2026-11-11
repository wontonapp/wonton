use serde::Deserialize;
use wonton_protocol::{ShareIdentityFailure, WontonDiscovery};

use super::{
    IdentityVerifier, MatrixServerName,
    dns::order_srv,
    https::{Destination, discovery_get},
};

impl IdentityVerifier {
    pub(super) async fn discover(
        &self,
        name: &MatrixServerName,
    ) -> Result<Vec<Destination>, ShareIdentityFailure> {
        let key = name.authority();
        if let Some(result) = self.matrix_cache.get(&key, self.clock.now()) {
            return Ok(result);
        }
        let result = self.discover_uncached(name).await?;
        self.matrix_cache
            .insert(key, result.clone(), self.clock.now());
        Ok(result)
    }

    async fn discover_uncached(
        &self,
        name: &MatrixServerName,
    ) -> Result<Vec<Destination>, ShareIdentityFailure> {
        if name.explicit_port.is_some() || name.ip().is_some() {
            return Ok(vec![Destination::direct(name.clone(), 8448)]);
        }
        let url = Destination::direct(name.clone(), 443).url("/.well-known/matrix/server")?;
        // Bound the optional well-known stage separately so its timeout still leaves time
        // for the specification's SRV/default-port fallback inside the total deadline.
        let well_known = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            discovery_get(self.dns.as_ref(), self.http.as_ref(), url),
        )
        .await;
        let delegated = match well_known {
            Ok(Ok(response)) if response.status.is_success() => {
                #[derive(Deserialize)]
                struct WellKnown {
                    #[serde(rename = "m.server")]
                    server: String,
                }
                serde_json::from_slice::<WellKnown>(&response.body)
                    .ok()
                    .and_then(|value| MatrixServerName::parse(&value.server).ok())
            }
            // Matrix discovery explicitly falls back on unavailable/invalid well-known.
            _ => None,
        };
        let authority = delegated.as_ref().unwrap_or(name);
        if authority.explicit_port.is_some() || authority.ip().is_some() {
            return Ok(vec![Destination::direct(authority.clone(), 8448)]);
        }
        for service in ["_matrix-fed._tcp", "_matrix._tcp"] {
            let query = format!("{service}.{}", authority.host);
            let Some(records) = self.dns.srv(&query).await? else {
                continue;
            };
            let mut destinations = Vec::new();
            for record in order_srv(records)? {
                // A root target is an explicit service-unavailable answer, not permission to
                // fall back to the identity host. SRV targets cannot delegate another port.
                let target = MatrixServerName::parse(&record.target)?;
                if target.explicit_port.is_some() || target.ip().is_some() || record.port == 0 {
                    return Err(ShareIdentityFailure::DiscoveryFailed);
                }
                destinations.push(Destination {
                    authority: authority.clone(),
                    target: target.host,
                    port: record.port,
                });
            }
            return Ok(destinations);
        }
        Ok(vec![Destination::direct(authority.clone(), 8448)])
    }

    /// Fetch a friend's sharing endpoint over verified HTTPS. None means no discovery document
    /// (404/410), not an authenticated identity. The caller must bind iroh to this exact public key.
    /// Endpoint hints are cached for one hour; no access tokens enter this cache.
    pub async fn discover_wonton(
        &self,
        server: &str,
    ) -> Result<Option<WontonDiscovery>, ShareIdentityFailure> {
        let name = MatrixServerName::parse(server)?;
        let key = name.authority();
        if let Some(result) = self.wonton_cache.get(&key, self.clock.now()) {
            return Ok(Some(result));
        }
        let operation = async {
            let url =
                Destination::direct(name, 443).url(wonton_protocol::WONTON_WELL_KNOWN_PATH)?;
            let response = discovery_get(self.dns.as_ref(), self.http.as_ref(), url).await?;
            if matches!(response.status.as_u16(), 404 | 410) {
                return Ok(None);
            }
            if !response.status.is_success() {
                return Err(ShareIdentityFailure::DiscoveryFailed);
            }
            let discovery: WontonDiscovery = serde_json::from_slice(&response.body)
                .map_err(|_| ShareIdentityFailure::DiscoveryFailed)?;
            validate_wonton(&discovery)?;
            self.wonton_cache
                .insert(key, discovery.clone(), self.clock.now());
            Ok(Some(discovery))
        };
        tokio::time::timeout(super::TOTAL_TIMEOUT, operation)
            .await
            .unwrap_or(Err(ShareIdentityFailure::DiscoveryFailed))
    }
}

fn validate_wonton(discovery: &WontonDiscovery) -> Result<(), ShareIdentityFailure> {
    let fail = ShareIdentityFailure::DiscoveryFailed;
    let endpoint: iroh::EndpointId = discovery.endpoint_id.parse().map_err(|_| fail)?;
    if endpoint.to_string() != discovery.endpoint_id || discovery.relay_urls.len() > 16 {
        return Err(fail);
    }
    for relay in &discovery.relay_urls {
        let url = url::Url::parse(relay).map_err(|_| fail)?;
        if url.scheme() != "https"
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
            || relay.len() > 2048
        {
            return Err(fail);
        }
    }
    Ok(())
}
