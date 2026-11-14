use std::sync::Arc;

use anyhow::Context;
use wonton_core::config::Config;
use wonton_core::friends::Friends;
use wonton_core::gateway::Gateway;
use wonton_core::gateway::sharing::{Sharing, SharingHandler};
use wonton_core::peers::Peers;
use wonton_core::roles::Roles;
use wonton_core::share_identity::{IdentityVerifier, MatrixServerName};
use wonton_core::shares::Shares;
use wonton_tunnel::federation::FederationForwarder;
use wonton_tunnel::sharing::{ShareLimits, ShareProtocol};

mod rate;
pub(crate) use rate::LimitedShares;

#[derive(Clone)]
pub(crate) struct Registries {
    pub friends: Arc<Friends>,
    pub shares: Arc<Shares>,
}

impl Registries {
    pub fn open(cfg: &Config, roles: Arc<Roles>) -> anyhow::Result<Self> {
        let friends = Arc::new(
            Friends::open(cfg.core_state_dir().join("friends.json"))
                .context("opening accepted Matrix friend registry")?,
        );
        let shares = Arc::new(
            Shares::open(
                cfg.core_state_dir().join("shares.json"),
                friends.clone(),
                roles,
            )
            .context("opening plugin share grants")?,
        );
        Ok(Self { friends, shares })
    }

    pub fn service(
        &self,
        peers: Option<Arc<Peers>>,
        forwarder: Option<Arc<FederationForwarder>>,
    ) -> anyhow::Result<Arc<Sharing>> {
        anyhow::ensure!(
            peers.is_none() || forwarder.is_some(),
            "paired sharing requires its iroh forwarder"
        );
        let lookup = Arc::new(move |name: &MatrixServerName| {
            let peers = peers.as_ref()?;
            paired_name(peers, name)?;
            forwarder.clone()
        });
        let mut verifier = IdentityVerifier::new(lookup)
            .map_err(|reason| anyhow::anyhow!("creating share identity verifier: {reason:?}"))?;
        if let Some(path) = std::env::var_os("WONTON_SHARE_CA_FILE") {
            let pem = std::fs::read(&path).context("reading explicitly trusted share CA file")?;
            let roots = reqwest::Certificate::from_pem_bundle(&pem)
                .context("parsing explicitly trusted share CA file")?;
            anyhow::ensure!(!roots.is_empty(), "share CA file contains no certificates");
            verifier = verifier.with_trusted_roots(roots);
        }
        Ok(Arc::new(Sharing::new(
            self.friends.clone(),
            self.shares.clone(),
            Arc::new(verifier),
        )))
    }
}

fn paired_name(peers: &Peers, name: &MatrixServerName) -> Option<String> {
    let names = peers.paired_names();
    names.borrow().iter().find_map(|candidate| {
        MatrixServerName::parse(candidate)
            .ok()
            .filter(|parsed| parsed.same_identity(name))
            .map(|_| candidate.clone())
    })
}

pub(crate) fn paired_target(
    peers: &Peers,
    input: &str,
) -> Option<(iroh::EndpointId, iroh::EndpointAddr)> {
    let parsed = MatrixServerName::parse(input).ok()?;
    let name = paired_name(peers, &parsed)?;
    peers.lookup(&name)
}

pub(crate) fn protocol(gateway: Arc<Gateway>) -> anyhow::Result<LimitedShares> {
    let limits = ShareLimits {
        max_active_streams: positive_env("WONTON_SHARE_MAX_STREAMS", 64)?,
    };
    let protocol = ShareProtocol::new(Arc::new(SharingHandler::new(gateway)), limits);
    Ok(LimitedShares::new(
        protocol,
        positive_env("WONTON_SHARE_CONNECTIONS_PER_MINUTE", 30)?,
    ))
}

fn positive_env(name: &str, default: u32) -> anyhow::Result<u32> {
    let value = match std::env::var(name) {
        Ok(value) => value
            .parse::<u32>()
            .with_context(|| format!("{name} must be a positive integer"))?,
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => return Err(error).with_context(|| format!("reading {name}")),
    };
    anyhow::ensure!(value > 0, "{name} must be a positive integer");
    Ok(value)
}
