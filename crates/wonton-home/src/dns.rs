use crate::{
    config::Config,
    store::{AcmeDns, Home},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub struct Dns {
    client: reqwest::Client,
    config: Config,
}

fn rrset(name: String, kind: &str, contents: Vec<String>, enabled: bool) -> Value {
    if enabled && !contents.is_empty() {
        json!({"name":name,"type":kind,"ttl":60,"changetype":"REPLACE",
            "records":contents.into_iter().map(|content| json!({"content":content,"disabled":false})).collect::<Vec<_>>()})
    } else {
        json!({"name":name,"type":kind,"changetype":"DELETE"})
    }
}

impl Dns {
    pub fn new(config: Config, client: reqwest::Client) -> Self {
        Self { client, config }
    }

    fn zone_url(&self) -> String {
        format!(
            "{}/api/v1/servers/localhost/zones/{}.",
            self.config.pdns_url.trim_end_matches('/'),
            self.config.zone
        )
    }

    pub async fn ensure_zone(&self) -> Result<()> {
        let response = self
            .client
            .get(self.zone_url())
            .header("X-API-Key", &self.config.pdns_key)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            self.client.post(format!("{}/api/v1/servers/localhost/zones", self.config.pdns_url.trim_end_matches('/')))
                .header("X-API-Key", &self.config.pdns_key)
                .json(&json!({"name":format!("{}.",self.config.zone),"kind":"Native","nameservers":[format!("ns.{}.",self.config.zone)]}))
                .send().await?.error_for_status()?;
        } else {
            response.error_for_status()?;
        }
        Ok(())
    }

    pub async fn register_acme(&self) -> Result<AcmeDns> {
        let account: AcmeDns = self
            .client
            .post(format!(
                "{}/register",
                self.config.acme_internal.trim_end_matches('/')
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let expected = format!("{}.auth.{}", account.subdomain, self.config.zone);
        if account.fulldomain != expected
            || account.username.is_empty()
            || account.password.is_empty()
            || !account
                .subdomain
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            bail!("acme-dns returned an invalid account");
        }
        Ok(account)
    }

    pub async fn reconcile(&self, label: &str, home: &Home) -> Result<()> {
        let domain = format!("{label}.{}.", self.config.zone);
        let active = !home.suspended && !home.deleted;
        let mut rrsets = vec![
            rrset(
                domain.clone(),
                "A",
                vec![self.config.relay_ip.to_string()],
                active,
            ),
            rrset(
                format!("*.{domain}"),
                "A",
                vec![self.config.relay_ip.to_string()],
                active,
            ),
            rrset(
                format!("_acme-challenge.{domain}"),
                "CNAME",
                vec![format!("{}.", home.acme_dns.fulldomain)],
                active,
            ),
            rrset(
                format!("_acme-challenge.lan.{domain}"),
                "CNAME",
                vec![format!("{}.", home.lan_acme_dns.fulldomain)],
                active,
            ),
        ];
        let caa = home.caa.as_ref().map(|binding| {
            format!(
                "{}; validationmethods=dns-01; accounturi={}",
                self.config.issuer, binding.account_uri
            )
        });
        let contents = caa
            .map(|value| {
                vec![
                    format!("0 issue \"{value}\""),
                    format!("0 issuewild \"{value}\""),
                ]
            })
            .unwrap_or_default();
        rrsets.push(rrset(domain, "CAA", contents, active));
        self.patch(rrsets).await
    }

    pub async fn patch(&self, rrsets: Vec<Value>) -> Result<()> {
        self.client
            .patch(self.zone_url())
            .header("X-API-Key", &self.config.pdns_key)
            .json(&json!({"rrsets":rrsets}))
            .send()
            .await
            .context("PowerDNS transport")?
            .error_for_status()
            .context("PowerDNS rejected DNS mutation")?;
        Ok(())
    }
}
