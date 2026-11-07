use reqwest::{Client, Url};
use serde::Deserialize;
use wonton_protocol::RegistryProblem;

use super::{network_problem, rejected};
use crate::l10n;
use crate::plugin::languages::Languages;

pub(super) struct Challenge {
    realm: Url,
    service: Option<String>,
    scope: Option<String>,
}

impl Challenge {
    pub(super) fn parse(header: &str) -> Option<Self> {
        let (scheme, mut rest) = header.trim().split_once(char::is_whitespace)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        let mut realm = None;
        let mut service = None;
        let mut scope = None;
        while !rest.trim().is_empty() {
            let (key, value, remaining) = parameter(rest)?;
            match key {
                key if key.eq_ignore_ascii_case("realm") => realm = Some(value),
                key if key.eq_ignore_ascii_case("service") => service = Some(value),
                key if key.eq_ignore_ascii_case("scope") => scope = Some(value),
                _ => {}
            }
            rest = remaining;
        }
        let realm = Url::parse(&realm?).ok()?;
        if !matches!(realm.scheme(), "https" | "http")
            || !realm.username().is_empty()
            || realm.password().is_some()
        {
            return None;
        }
        Some(Self {
            realm,
            service,
            scope,
        })
    }

    pub(super) async fn token(
        self,
        client: &Client,
        langs: &Languages,
    ) -> Result<String, RegistryProblem> {
        let mut url = self.realm;
        {
            let mut query = url.query_pairs_mut();
            if let Some(service) = self.service {
                query.append_pair("service", &service);
            }
            if let Some(scope) = self.scope {
                query.append_pair("scope", &scope);
            }
        }
        // 不在 Client 上设置默认 Authorization，令牌端点及其跳转始终匿名访问。
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| network_problem(&e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejected(
                langs,
                status,
                l10n::text(langs, "plugin-settings-mirror-token-request-rejected", &[]).as_str(),
            ));
        }
        let bytes = response.bytes().await.map_err(|e| network_problem(&e))?;
        let token: Token = serde_json::from_slice(&bytes).map_err(|error| {
            rejected(
                langs,
                status,
                l10n::text(
                    langs,
                    "plugin-settings-mirror-token-response-invalid",
                    &[("error", error.to_string().into())],
                )
                .as_str(),
            )
        })?;
        token
            .token
            .filter(|token| !token.is_empty())
            .or_else(|| token.access_token.filter(|token| !token.is_empty()))
            .ok_or_else(|| {
                rejected(
                    langs,
                    status,
                    l10n::text(langs, "plugin-settings-mirror-token-missing", &[]).as_str(),
                )
            })
    }
}

#[derive(Deserialize)]
struct Token {
    token: Option<String>,
    access_token: Option<String>,
}

fn parameter(input: &str) -> Option<(&str, String, &str)> {
    let (key, rest) = input.trim_start().split_once('=')?;
    let key = key.trim();
    let rest = rest.trim_start();
    if !rest.starts_with('"') {
        let (value, remaining) = rest.split_once(',').unwrap_or((rest, ""));
        return Some((key, value.trim().to_owned(), remaining));
    }
    let mut escaped = false;
    let mut value = String::new();
    for (offset, ch) in rest[1..].char_indices() {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            let remaining = rest[offset + 2..].trim_start();
            let remaining = if remaining.is_empty() {
                remaining
            } else {
                remaining.strip_prefix(',')?
            };
            return Some((key, value, remaining));
        } else {
            value.push(ch);
        }
    }
    None
}
