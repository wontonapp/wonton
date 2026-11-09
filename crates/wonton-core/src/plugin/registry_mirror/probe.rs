use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::{ACCEPT, ACCEPT_ENCODING, WWW_AUTHENTICATE};
use reqwest::{Client, Response, StatusCode};
use sha2::{Digest, Sha256};
use wonton_protocol::{RegistryFailure, RegistryProbe, RegistryProblem};

use crate::l10n;
use crate::plugin::languages::Languages;

#[path = "auth.rs"]
mod auth;

const TIMEOUT: Duration = Duration::from_secs(5);
const ACCEPT_MANIFEST: &str = concat!(
    "application/vnd.docker.distribution.manifest.v2+json, ",
    "application/vnd.docker.distribution.manifest.list.v2+json, ",
    "application/vnd.oci.image.manifest.v1+json, ",
    "application/vnd.oci.image.index.v1+json"
);

pub(super) async fn probe(mirror: &str, langs: &Languages) -> RegistryProbe {
    let digest = crate::plugin::egress::gost::IMAGE
        .split_once('@')
        .expect("gost 镜像必须按摘要固定")
        .1;
    let url = format!("https://{mirror}/v2/gogost/gost/manifests/{digest}");
    probe_url(&url, digest, TIMEOUT, true, langs).await
}

async fn probe_url(
    url: &str,
    digest: &str,
    timeout: Duration,
    https_only: bool,
    langs: &Languages,
) -> RegistryProbe {
    let probed_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, fetch(url, digest, https_only, langs)).await;
    let problem = match result {
        Ok(result) => result.err(),
        Err(error) => Some(problem(
            RegistryFailure::Unreachable,
            l10n::text(
                langs,
                "plugin-settings-mirror-probe-timeout",
                &[
                    ("ms", timeout.as_millis().to_string().into()),
                    ("error", error.to_string().into()),
                ],
            ),
        )),
    };
    RegistryProbe {
        probed_at_ms,
        latency_ms: problem
            .is_none()
            .then(|| started.elapsed().as_millis().min(u32::MAX as u128) as u32),
        problem,
    }
}

async fn fetch(
    url: &str,
    digest: &str,
    https_only: bool,
    langs: &Languages,
) -> Result<(), RegistryProblem> {
    let client = Client::builder()
        .no_proxy()
        .https_only(https_only)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .user_agent("docker/28.0.0 go/go1.23.0 os/linux arch/amd64")
        .build()
        .map_err(|e| network_problem(&e))?;
    let mut response = manifest(&client, url, None).await?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let challenge = response
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(auth::Challenge::parse)
            .ok_or_else(|| {
                rejected(
                    langs,
                    response.status(),
                    l10n::text(langs, "plugin-settings-mirror-no-challenge", &[]).as_str(),
                )
            })?;
        // 重试发给实际发出挑战的地址，不能把重定向目标的令牌交给原始主机。
        let retry_url = response.url().clone();
        drop(response);
        let token = challenge.token(&client, langs).await?;
        response = manifest(&client, retry_url.as_str(), Some(&token)).await?;
    }
    if !response.status().is_success() {
        return Err(rejected(
            langs,
            response.status(),
            l10n::text(langs, "plugin-settings-mirror-manifest-rejected", &[]).as_str(),
        ));
    }
    let mut hash = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| network_problem(&e))? {
        hash.update(&chunk);
    }
    let actual = format!("sha256:{:x}", hash.finalize());
    if actual != digest {
        return Err(problem(
            RegistryFailure::DigestMismatch,
            l10n::text(
                langs,
                "plugin-settings-mirror-manifest-digest",
                &[("digest", digest.into()), ("actual", actual.into())],
            ),
        ));
    }
    Ok(())
}

async fn manifest(
    client: &Client,
    url: &str,
    token: Option<&str>,
) -> Result<Response, RegistryProblem> {
    let mut request = client
        .get(url)
        .header(ACCEPT, ACCEPT_MANIFEST)
        .header(ACCEPT_ENCODING, "identity");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    // reqwest 0.13 在重定向的主机、端口或协议变化时剥离 Authorization。
    request.send().await.map_err(|e| network_problem(&e))
}

fn network_problem(error: &reqwest::Error) -> RegistryProblem {
    use std::error::Error;

    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    problem(RegistryFailure::Unreachable, message)
}

fn rejected(langs: &Languages, status: StatusCode, detail: &str) -> RegistryProblem {
    problem(
        RegistryFailure::Rejected,
        l10n::text(
            langs,
            "plugin-settings-mirror-status",
            &[
                ("status", status.as_str().into()),
                ("detail", detail.into()),
            ],
        ),
    )
}

fn problem(reason: RegistryFailure, message: String) -> RegistryProblem {
    RegistryProblem { reason, message }
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
