use crate::{service::App, store::now};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

// Gregorian civil-from-days, with the Unix epoch at day zero; no local timezone.
pub fn utc_month(seconds: u64) -> u64 {
    let z = seconds / 86400 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    year * 12 + month - 1
}

fn sample(line: &str) -> Result<Option<(&str, String, u64)>> {
    let Some((metric, rest)) = line.split_once('{') else {
        return Ok(None);
    };
    if !matches!(
        metric,
        "frp_server_traffic_in"
            | "frp_server_traffic_out"
            | "frp_server_connection_counts"
            | "frp_server_proxy_counts_detailed"
    ) {
        return Ok(None);
    }
    let (labels, number) = rest.split_once('}').context("malformed metrics sample")?;
    let name = labels
        .split(',')
        .find_map(|label| {
            label
                .strip_prefix("name=\"")
                .and_then(|s| s.strip_suffix('"'))
        })
        .context("missing proxy metric name")?;
    let value: f64 = number.trim().parse()?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        bail!("invalid counter sample");
    }
    Ok(Some((metric, name.to_owned(), value as u64)))
}

impl App {
    pub async fn poll_metrics(&self) -> Result<()> {
        let response = self
            .client
            .get(&self.config.metrics_url)
            .basic_auth(
                &self.config.metrics_user,
                Some(&self.config.metrics_password),
            )
            .send()
            .await?
            .error_for_status()?;
        let text = response.text().await?;
        if !text.contains("frp_server_client_counts") {
            bail!("frps metrics unavailable");
        }
        let clock = now();
        let month = utc_month(clock);
        let mut store = self.store.lock().await;
        let mut runtime = self.runtime.lock().await;
        let mut connections = BTreeMap::<String, u64>::new();
        let mut proxies = BTreeMap::<String, String>::new();
        for home in store.data.homes.values_mut() {
            if home.usage_month != month {
                home.usage_month = month;
                home.usage_bytes = 0;
            }
        }
        for line in text.lines() {
            let Some((metric, name, value)) = sample(line)? else {
                continue;
            };
            let Some((label, _)) = name.split_once('.') else {
                continue;
            };
            if !store.data.homes.contains_key(label) {
                continue;
            }
            match metric {
                "frp_server_connection_counts" => {
                    *connections.entry(label.to_owned()).or_default() += value;
                }
                "frp_server_proxy_counts_detailed" if value > 0 => {
                    proxies.insert(name.clone(), label.to_owned());
                }
                "frp_server_traffic_in" | "frp_server_traffic_out" => {
                    let key = format!("{metric}:{name}");
                    let previous = store.data.counters.insert(key, value).unwrap_or(0);
                    let delta = if value >= previous {
                        value - previous
                    } else {
                        value
                    };
                    let home = store.data.homes.get_mut(label).expect("known home");
                    home.usage_bytes = home.usage_bytes.saturating_add(delta);
                }
                _ => {}
            }
        }
        store.save()?;
        runtime.connections = connections;
        // A rejected/conflicting frps registration has no CloseProxy event. Expire only
        // unobserved reservations; published proxies remain charged across scrapes.
        runtime.proxy_reservations.retain(|name, accepted| {
            !proxies.contains_key(name) && clock.saturating_sub(*accepted) <= 90
        });
        let reservations = &runtime.proxy_reservations;
        let mut active = proxies;
        for name in reservations.keys() {
            if let Some((label, _)) = name.split_once('.') {
                active.insert(name.clone(), label.to_owned());
            }
        }
        runtime.proxies = active;
        runtime.metrics_at = clock;
        Ok(())
    }
}
