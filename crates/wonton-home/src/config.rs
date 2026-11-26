use crate::store::hash;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{env, net::Ipv4Addr, path::PathBuf};
use url::Url;

#[derive(Clone, Serialize)]
pub struct Relay {
    pub host: String,
    pub port: u16,
    pub server_name: String,
}

/// Operator-owned frpc identity used for the public entry vhosts (push, relay,
/// omarchy, bootstrap) that do not belong to a registered home.
#[derive(Clone)]
pub struct System {
    pub domains: Vec<String>,
    pub token_hash: String,
}

#[derive(Clone)]
pub struct Config {
    pub listen: String,
    pub hook_listen: String,
    pub state: PathBuf,
    pub zone: String,
    pub relay_ip: Ipv4Addr,
    pub relay: Relay,
    pub acme_url: String,
    pub acme_internal: String,
    pub pdns_url: String,
    pub pdns_key: String,
    pub issuer: String,
    pub metrics_url: String,
    pub metrics_user: String,
    pub metrics_password: String,
    pub system: Option<System>,
}

pub fn required(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing {name}"))
}

fn secret(name: &str) -> Result<String> {
    let path = required(name)?;
    let value = std::fs::read_to_string(path)?.trim().to_owned();
    if value.is_empty() {
        bail!("empty secret {name}");
    }
    Ok(value)
}

pub fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let zone = required("WONTON_HOME_ZONE")?;
        if !zone.split('.').all(valid_label) {
            bail!("invalid zone");
        }
        let issuer = required("WONTON_HOME_CAA_ISSUER")?;
        if !issuer.split('.').all(valid_label) {
            bail!("invalid CAA issuer");
        }
        let acme_url = required("WONTON_HOME_ACME_URL")?;
        let parsed = Url::parse(&acme_url)?;
        if parsed.scheme() != "https" || parsed.query().is_some() || parsed.fragment().is_some() {
            bail!("ACME update URL must be HTTPS without query/fragment");
        }
        let system = match (
            env::var("WONTON_HOME_SYSTEM_TOKEN_FILE").ok(),
            env::var("WONTON_HOME_SYSTEM_DOMAINS").ok(),
        ) {
            (None, None) => None,
            (Some(path), Some(domains)) => {
                let token = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading WONTON_HOME_SYSTEM_TOKEN_FILE {path}"))?;
                let token = token.trim();
                if token.is_empty() {
                    bail!("empty system token");
                }
                let domains: Vec<String> = domains
                    .split(',')
                    .map(str::trim)
                    .filter(|domain| !domain.is_empty())
                    .map(str::to_owned)
                    .collect();
                if domains.is_empty() {
                    bail!("empty WONTON_HOME_SYSTEM_DOMAINS");
                }
                for domain in &domains {
                    if domain.split('.').count() < 2 || !domain.split('.').all(valid_label) {
                        bail!("invalid system domain {domain}");
                    }
                }
                Some(System {
                    domains,
                    token_hash: hash(token.as_bytes()),
                })
            }
            (Some(_), None) => bail!("WONTON_HOME_SYSTEM_DOMAINS missing"),
            (None, Some(_)) => bail!("WONTON_HOME_SYSTEM_TOKEN_FILE missing"),
        };
        Ok(Self {
            listen: env::var("WONTON_HOME_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into()),
            hook_listen: env::var("WONTON_HOME_HOOK_LISTEN")
                .unwrap_or_else(|_| "127.0.0.1:8081".into()),
            state: required("WONTON_HOME_STATE")?.into(),
            zone,
            relay_ip: required("WONTON_HOME_RELAY_IP")?.parse()?,
            relay: Relay {
                host: required("WONTON_HOME_RELAY_HOST")?,
                port: required("WONTON_HOME_RELAY_PORT")?.parse()?,
                server_name: required("WONTON_HOME_RELAY_SERVER_NAME")?,
            },
            acme_url,
            acme_internal: required("WONTON_HOME_ACME_INTERNAL_URL")?,
            pdns_url: required("WONTON_HOME_PDNS_URL")?,
            pdns_key: secret("WONTON_HOME_PDNS_KEY_FILE")?,
            system,
            issuer,
            metrics_url: required("WONTON_HOME_FRPS_METRICS_URL")?,
            metrics_user: required("WONTON_HOME_FRPS_METRICS_USER")?,
            metrics_password: secret("WONTON_HOME_FRPS_METRICS_PASSWORD_FILE")?,
        })
    }
}
