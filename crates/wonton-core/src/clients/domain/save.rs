//! 保存设置：校验、固定后缀和凭据规则、变更标记。

use std::sync::Arc;

use anyhow::Result;
use wonton_protocol::PutDomainRequest;

use super::acme::{validate_credentials, validate_provider};
use super::settings::invalid;
use super::{Domain, DomainError};
use crate::entry_settings::EntrySettings;

pub const SUFFIX_CHANGED_HEADER: &str = "X-Wonton-Clients-Notice";
pub const PREVIOUS_SUFFIX_HEADER: &str = "X-Wonton-Previous-Suffix";

/// 一次保存的结果：后缀变了要给壳一个“第三方 App 要重填”的标记。
pub struct Saved {
    pub previous_suffix: Option<String>,
    pub suffix_changed: bool,
    pub provider_changed: bool,
}

impl Domain {
    /// 保存设置并排一次签发。`suffix` 在服务器名定死时只能是同一个值，改不了。
    /// 凭据为空时沿用已存的；家名服务的安装本来就不用服务商和凭据，两者也都可空。
    pub async fn save(self: &Arc<Self>, request: PutDomainRequest) -> Result<Saved, DomainError> {
        if !request.accept_terms {
            return Err(DomainError::InvalidKey("clients-domain-terms-required"));
        }
        let settings = EntrySettings::load(&self.install_dir).map_err(invalid)?;
        // 主人自己放了 site.pem 的安装不跑 ACME：收下设置也没人签，直接说清楚。
        if settings
            .as_ref()
            .is_some_and(|settings| settings.external_certificate)
        {
            return Err(DomainError::ConflictKey(
                "clients-domain-external-certificate",
            ));
        }
        let registered = settings
            .as_ref()
            .is_some_and(|settings| settings.registration.is_some());
        if !(registered && request.provider.is_empty()) {
            validate_provider(&request.provider).map_err(invalid)?;
        }
        validate_credentials(&request.credentials).map_err(invalid)?;
        let fixed = self.fixed_suffix().map_err(invalid)?;
        let provider = request.provider;
        let credentials: std::collections::BTreeMap<String, String> = request
            .credentials
            .into_iter()
            .map(|entry| (entry.name, entry.value.expose().to_owned()))
            .collect();
        let saved = self
            .apply(|store| {
                let previous = store.suffix.clone();
                let suffix = match &fixed {
                    Some(fixed) => {
                        if request
                            .suffix
                            .as_deref()
                            .is_some_and(|value| value != fixed)
                        {
                            return Err(DomainError::ConflictKey("clients-domain-fixed-suffix"));
                        }
                        fixed.clone()
                    }
                    None => {
                        let suffix = request
                            .suffix
                            .clone()
                            .or_else(|| previous.clone())
                            .ok_or(DomainError::InvalidKey("clients-domain-suffix-required"))?;
                        if let Some(id) = suffix_problem(&suffix) {
                            return Err(DomainError::InvalidKey(id));
                        }
                        suffix
                    }
                };
                let changed = previous.as_deref().is_some_and(|old| old != suffix);
                // 空服务商＝沿用现状（家名安装的常态），不算变更，不然每次保存都会误重置签发准备。
                let effective_provider = if provider.is_empty() {
                    store.provider.clone()
                } else {
                    Some(provider.clone())
                };
                let provider_changed = store.provider != effective_provider;
                // 首次配置或换服务商必须一起提交凭据，否则设置收下了却永远不签，也没人知道为什么。
                if credentials.is_empty()
                    && !(registered && provider.is_empty())
                    && (provider_changed || !store.has_credentials())
                {
                    return Err(DomainError::InvalidKey(
                        "clients-domain-credentials-required",
                    ));
                }
                store.suffix = Some(suffix);
                if let Some(provider) = effective_provider {
                    store.provider = Some(provider);
                }
                if !credentials.is_empty() {
                    store.credentials = credentials.clone();
                    // App 提交的凭据取代装机给的文件，别再让旧文件抢着生效。
                    store.env_file = None;
                }
                store.terms_accepted = true;
                Ok(Saved {
                    previous_suffix: changed.then_some(previous).flatten(),
                    suffix_changed: changed,
                    provider_changed,
                })
            })
            .await?;
        let credentials_supplied = !credentials.is_empty();
        if saved.suffix_changed || saved.provider_changed {
            // 删准备状态要和进行中的签发互斥：签发正用着它们。
            let _serial = self.issuance.lock().await;
            self.reset_prepared();
        }
        // 换了名字或服务商要重签；只换凭据时准备状态还在、证书也新，得强制一次去验新凭据。
        self.spawn_issue(credentials_supplied || saved.suffix_changed || saved.provider_changed);
        Ok(saved)
    }
}

/// 后缀不合法时给哪条本地化说法；规则和 `validate_domain` 一致，只是按 App 的语言分。
fn suffix_problem(suffix: &str) -> Option<&'static str> {
    match wonton_protocol::suffix_problem(suffix)? {
        wonton_protocol::SuffixProblem::Invalid => Some("clients-domain-suffix-invalid"),
        wonton_protocol::SuffixProblem::Ip => Some("clients-domain-suffix-ip"),
    }
}
