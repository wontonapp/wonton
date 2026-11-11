use std::{future::Future, net::IpAddr, pin::Pin};

use hickory_resolver::{
    TokioResolver,
    config::{LookupIpStrategy, ResolveHosts},
    proto::rr::RData,
};
use wonton_protocol::ShareIdentityFailure;

use super::{MAX_ADDRESSES, MAX_SRV_RECORDS};

pub type IdentityFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// DNS seam; implementations return addresses, never HTTP clients or unchecked URLs.
/// `None` means NXDOMAIN/NODATA only. Transient SRV errors must not trigger 8448 fallback.
pub trait IdentityDns: Send + Sync {
    fn addresses<'a>(
        &'a self,
        host: &'a str,
    ) -> IdentityFuture<'a, Result<Vec<IpAddr>, ShareIdentityFailure>>;
    fn srv<'a>(
        &'a self,
        query: &'a str,
    ) -> IdentityFuture<'a, Result<Option<Vec<SrvRecord>>, ShareIdentityFailure>>;
}

pub(super) struct SystemDns(TokioResolver);

impl SystemDns {
    pub(super) fn new() -> Result<Self, ShareIdentityFailure> {
        let mut builder =
            TokioResolver::builder_tokio().map_err(|_| ShareIdentityFailure::DiscoveryFailed)?;
        let opts = builder.options_mut();
        opts.use_hosts_file = ResolveHosts::Never;
        opts.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        opts.cache_size = 256;
        opts.attempts = 1;
        opts.timeout = std::time::Duration::from_secs(3);
        builder
            .build()
            .map(Self)
            .map_err(|_| ShareIdentityFailure::DiscoveryFailed)
    }
}

impl IdentityDns for SystemDns {
    fn addresses<'a>(
        &'a self,
        host: &'a str,
    ) -> IdentityFuture<'a, Result<Vec<IpAddr>, ShareIdentityFailure>> {
        Box::pin(async move {
            let lookup = self
                .0
                .lookup_ip(format!("{host}."))
                .await
                .map_err(|_| ShareIdentityFailure::DiscoveryFailed)?;
            let addresses: Vec<_> = lookup.iter().take(MAX_ADDRESSES + 1).collect();
            if addresses.len() > MAX_ADDRESSES {
                return Err(ShareIdentityFailure::DiscoveryFailed);
            }
            Ok(addresses)
        })
    }

    fn srv<'a>(
        &'a self,
        query: &'a str,
    ) -> IdentityFuture<'a, Result<Option<Vec<SrvRecord>>, ShareIdentityFailure>> {
        Box::pin(async move {
            let lookup = match self.0.srv_lookup(format!("{query}.")).await {
                Ok(lookup) => lookup,
                Err(error) if error.is_no_records_found() => return Ok(None),
                Err(_) => return Err(ShareIdentityFailure::DiscoveryFailed),
            };
            let mut records = Vec::new();
            for record in lookup.answers() {
                if let RData::SRV(srv) = &record.data {
                    if records.len() == MAX_SRV_RECORDS {
                        return Err(ShareIdentityFailure::DiscoveryFailed);
                    }
                    records.push(SrvRecord {
                        priority: srv.priority,
                        weight: srv.weight,
                        port: srv.port,
                        target: srv.target.to_utf8().trim_end_matches('.').to_owned(),
                    });
                }
            }
            Ok((!records.is_empty()).then_some(records))
        })
    }
}

/// RFC 2782 order: ascending priority, weighted random selection within each priority.
/// Bounded by MAX_SRV_RECORDS before any target is resolved.
pub(super) fn order_srv(
    mut records: Vec<SrvRecord>,
) -> Result<Vec<SrvRecord>, ShareIdentityFailure> {
    if records.is_empty() || records.len() > MAX_SRV_RECORDS {
        return Err(ShareIdentityFailure::DiscoveryFailed);
    }
    records.sort_by_key(|r| (r.priority, r.weight != 0));
    let mut ordered = Vec::with_capacity(records.len());
    while !records.is_empty() {
        let end = records
            .iter()
            .take_while(|r| r.priority == records[0].priority)
            .count();
        let total: u64 = records[..end].iter().map(|r| u64::from(r.weight)).sum();
        let mut random = [0; 8];
        getrandom::fill(&mut random).map_err(|_| ShareIdentityFailure::DiscoveryFailed)?;
        let chosen = u64::from_ne_bytes(random) % (total + 1);
        let mut sum = 0;
        let index = records[..end]
            .iter()
            .position(|r| {
                sum += u64::from(r.weight);
                sum >= chosen
            })
            .unwrap_or(0);
        ordered.push(records.remove(index));
    }
    Ok(ordered)
}
