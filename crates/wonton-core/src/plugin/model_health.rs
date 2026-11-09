//! 默认路由的探测：模型列表只说明路由存在，最小补全才验证服务商的凭据与连接。
//! 结论写进使用方的凭据文件，详情页组装成轻提示（ADR-0055），不碰插件状态。

use std::time::Duration;

use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, Permission, PluginId, PluginKind};
use super::models::{ModelsRoute, unavailable};
use super::store::{create_private_dir, read_json, write_json};
use crate::http::{HttpError, RequestBody};
use crate::l10n;
use crate::secret::random_token;

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const UPSTREAM_RECHECK: Duration = Duration::from_secs(5 * 60);
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

/// 一次探测对“接上网关之后”的路由结论；网关不在由详情页现场推导（见 `models_hint`）。
/// 文字是探测时按主人语言写好的成品，与凭据文件里 `problem` 的语言时点一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum RouteProbe {
    /// 网关在运行，但模型列表拿不到：事实上没接上，类别仍是 `models_not_connected`。
    Unreachable(String),
    /// 列表里没有默认模型：主人在“模型设置”里选服务商或加模型。
    NoDefaultModel,
    /// 最小补全被服务商拒绝或失败，原因随轻提示的 detail 带出。
    ProviderError(String),
}

/// 计费节奏：30 秒巡检只读模型列表，不花一块钱；
/// 最小补全只在核心启动首轮、使用方刚安装、刚启动时各做一次，服务商出错后每 5 分钟复查，
/// 恢复即停。缺模型时不发补全；每 30 秒补全一次是每天约 2880 次的用量污染，不做健康稳态验证。
#[derive(Default)]
pub(super) struct ProbeSchedule {
    startup_checked: bool,
    upstream_error: Option<(String, Instant)>,
}

pub(super) fn needs_default_model(manifest: &Manifest) -> bool {
    manifest.kind == PluginKind::Agent || manifest.permissions.contains(&Permission::Models)
}

impl PluginManager {
    /// 详情页的轻提示（ADR-0043：核心给类别，App 按类别写文字）：声明了 `models` 的插件没接上网关时给。
    /// 网关不在、没就绪，或凭据还没签发（包括签发失败，原因在 detail 里）都算没接上；
    /// 接上之后看默认路由的探测结论。
    pub(in crate::plugin) fn models_hint(
        &self,
        id: &PluginId,
        manifest: &Manifest,
    ) -> Option<wonton_protocol::PluginHint> {
        use super::models::ProviderState;
        use wonton_protocol::PluginHintCategory as Category;
        if !super::models::uses_models(manifest) {
            return None;
        }
        if !matches!(self.models_provider(), ProviderState::Ready(_)) {
            // 网关不在或没就绪：详情页现场推导，没有可带的原始原因（与 ADR-0055 相同）。
            return Some(wonton_protocol::PluginHint {
                category: Category::ModelsNotConnected,
                detail: None,
                provider: None,
                capability: None,
            });
        }
        let (has_keys, problem, route) = self.store.model_keys().hint_basis(id);
        if problem.is_some() || !has_keys {
            return Some(wonton_protocol::PluginHint {
                category: Category::ModelsNotConnected,
                detail: problem,
                provider: None,
                capability: None,
            });
        }
        route.map(|route| match route {
            RouteProbe::Unreachable(detail) => {
                // 网关在运行但模型列表拿不到：事实上没接上，归 models_not_connected（ADR-0055
                // 只写了“没装、没就绪、签发/验证失败”，这里按“列表都拿不到=没接上”归类）。
                wonton_protocol::PluginHint {
                    category: Category::ModelsNotConnected,
                    detail: Some(detail),
                    provider: None,
                    capability: None,
                }
            }
            RouteProbe::NoDefaultModel => wonton_protocol::PluginHint {
                category: Category::ModelsNoDefaultModel,
                detail: None,
                provider: None,
                capability: None,
            },
            RouteProbe::ProviderError(detail) => wonton_protocol::PluginHint {
                category: Category::ModelsProviderError,
                detail: Some(detail),
                provider: None,
                capability: None,
            },
        })
    }

    /// 刚安装的使用方：补全节奏里“刚安装”的那一次（网关本身的清单不申请 `models`，不会进来）。
    pub(super) async fn probe_fresh_consumer(&self, manifest: &Manifest) {
        if !needs_default_model(manifest) {
            return;
        }
        self.check_models_once(vec![manifest.id.clone()]).await;
    }
}

enum ProbeOutcome {
    /// `None`：网关没装或没就绪，详情页现场推导；`Some`：网关在运行但模型列表拿不到。
    GatewayUnavailable(Option<String>),
    MissingModel,
    ProviderError(String),
    Available,
}

impl ProbeOutcome {
    fn into_probe(self) -> Option<RouteProbe> {
        match self {
            Self::GatewayUnavailable(detail) => detail.map(RouteProbe::Unreachable),
            Self::MissingModel => Some(RouteProbe::NoDefaultModel),
            Self::ProviderError(reason) => Some(RouteProbe::ProviderError(reason)),
            Self::Available => None,
        }
    }
}

impl PluginManager {
    async fn default_model_target(
        &self,
        langs: &Languages,
    ) -> Result<(ModelsRoute, String), ProbeOutcome> {
        let Some(route) = self.models_route() else {
            return Err(ProbeOutcome::GatewayUnavailable(None));
        };
        let (key, catalog) = self
            .model_probe_catalog(&route, langs)
            .await
            .map_err(|e| ProbeOutcome::GatewayUnavailable(Some(e.to_string())))?;
        let models = catalog.get("data").and_then(Value::as_array);
        let Some(models) = models else {
            return Err(ProbeOutcome::GatewayUnavailable(Some(l10n::text(
                langs,
                "plugin-settings-models-list-malformed",
                &[("provider", route.id.as_str().into())],
            ))));
        };
        if !models
            .iter()
            .any(|model| model["id"].as_str() == Some(route.spec.default_model.as_str()))
        {
            return Err(ProbeOutcome::MissingModel);
        }
        Ok((route, key))
    }

    async fn scheduled_model_probe(
        &self,
        force: bool,
        now: Instant,
        langs: &Languages,
    ) -> ProbeOutcome {
        // 跨安装、启动与巡检串行安排：同一轮不会并发重复发计费请求。
        let mut schedule = self.store.model_keys().probe.lock().await;
        let first_watch = !schedule.startup_checked;
        schedule.startup_checked = true;
        let (route, key) = match self.default_model_target(langs).await {
            Ok(target) => target,
            Err(outcome) => return outcome,
        };
        let retry_due = schedule
            .upstream_error
            .as_ref()
            .is_some_and(|(_, at)| now >= *at);
        if !force && !first_watch && !retry_due {
            return match &schedule.upstream_error {
                Some((reason, _)) => ProbeOutcome::ProviderError(reason.clone()),
                None => ProbeOutcome::Available,
            };
        }
        let outcome = self.probe_completion(&route, &key, langs).await;
        match &outcome {
            ProbeOutcome::ProviderError(reason) => {
                schedule.upstream_error = Some((reason.clone(), now + UPSTREAM_RECHECK));
            }
            ProbeOutcome::Available => schedule.upstream_error = None,
            // 网关侧的问题（不可达、缺模型）不是服务商错误：清掉复查计时，下一轮巡检重看。
            _ => schedule.upstream_error = None,
        }
        outcome
    }

    /// 最小补全：一条用户消息、非流式、`max_tokens: 1`。用独立的健康凭据，可能计费。
    async fn probe_completion(
        &self,
        route: &ModelsRoute,
        key: &str,
        langs: &Languages,
    ) -> ProbeOutcome {
        let path = route.spec.apis.openai.trim_end_matches('/');
        let url = format!("http://{}{path}/chat/completions", route.upstream);
        let body = json!({
            "model": route.spec.default_model,
            "messages": [{"role": "user", "content": "Hi"}],
            "max_tokens": 1,
            "stream": false,
        });
        let outcome = match self
            .http
            .request_json(
                Method::POST,
                &url,
                Some(key),
                RequestBody::Json(&body),
                PROBE_TIMEOUT,
            )
            .await
        {
            Ok((StatusCode::OK, body))
                if body["choices"].as_array().is_some_and(|choices| {
                    choices
                        .iter()
                        .any(|choice| choice["message"]["role"] == "assistant")
                }) =>
            {
                ProbeOutcome::Available
            }
            Ok((StatusCode::OK, _)) => ProbeOutcome::ProviderError(l10n::text(
                langs,
                "plugin-settings-models-provider-invalid-reply",
                &[],
            )),
            Ok((status, _)) => ProbeOutcome::ProviderError(l10n::text(
                langs,
                "plugin-settings-models-provider-error",
                &[("error", format!("HTTP {status}").into())],
            )),
            Err(error @ (HttpError::Connect(_) | HttpError::BadUrl(_))) => {
                ProbeOutcome::GatewayUnavailable(Some(l10n::text(
                    langs,
                    "plugin-settings-models-unreachable",
                    &[
                        ("provider", route.id.as_str().into()),
                        ("error", error.to_string().into()),
                    ],
                )))
            }
            Err(error) => ProbeOutcome::ProviderError(l10n::text(
                langs,
                "plugin-settings-models-provider-error",
                &[("error", error.to_string().into())],
            )),
        };
        if let ProbeOutcome::ProviderError(reason) = &outcome {
            tracing::debug!(gateway = %route.id, reason, "默认模型的最小补全失败");
        }
        outcome
    }

    /// 运行中的模型使用方共用一轮探测结果（30 秒巡检）：只更新轻提示的依据，不碰插件状态。
    pub(super) async fn watch_models(&self, ids: Vec<PluginId>) {
        self.watch_models_at(ids, false, Instant::now()).await;
    }

    /// 一次性事件（安装、启动、核心启动首轮）后的探测：绕过巡检节流，立即发一次最小补全。
    pub(super) async fn check_models_once(&self, ids: Vec<PluginId>) {
        self.watch_models_at(ids, true, Instant::now()).await;
    }

    /// `now` 显式传入：测试用可控时钟断言五分钟复查的边界。
    pub(super) async fn watch_models_at(&self, ids: Vec<PluginId>, force: bool, now: Instant) {
        let consumers = self.retain_model_consumers(ids).await;
        if consumers.is_empty() {
            return;
        }
        let outcome = self
            .scheduled_model_probe(force, now, &self.owner_languages())
            .await;
        self.record_probe(consumers, outcome).await;
    }

    async fn retain_model_consumers(&self, ids: Vec<PluginId>) -> Vec<PluginId> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        ids.into_iter()
            .filter(|id| {
                plugins.get(id).is_some_and(|entry| {
                    entry.status == super::manager::PluginStatus::Running
                        && entry.listed.as_ref().is_some_and(|listed| {
                            listed.manifest.permissions.contains(&Permission::Models)
                        })
                })
            })
            .collect()
    }

    async fn record_probe(&self, ids: Vec<PluginId>, outcome: ProbeOutcome) {
        let probe = outcome.into_probe();
        for id in ids {
            self.store.model_keys().update_route(&id, probe.clone());
        }
    }

    /// 安装预览的提示（不挡安装）：默认路由还没有模型时提一句先接模型。只查模型列表，
    /// 不发可能计费的补全（见 `scheduled_model_probe` 的节奏约定）。
    pub(in crate::plugin) async fn model_route_notice(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Option<wonton_protocol::CardLine> {
        if !needs_default_model(manifest) {
            return None;
        }
        match self.default_model_target(langs).await {
            Err(ProbeOutcome::MissingModel) => {}
            _ => return None,
        }
        Some(wonton_protocol::CardLine {
            text: l10n::text(langs, "plugin-settings-models-connect-first", &[]),
            emphasis: true,
        })
    }
    /// 同 [`Self::issue_verified`]，但把验证时的模型列表带回来：默认路由的探测靠它判断
    /// 默认模型在不在（见 `model_health`）。
    pub(super) async fn issue_verified_catalog(
        &self,
        route: &ModelsRoute,
        name: &str,
        value: &str,
        langs: &Languages,
    ) -> Result<serde_json::Value, InstallError> {
        let issue = &route.spec.credentials.issue;
        self.run_command(route, issue, name, Some(value), langs)
            .await?;
        if let Some(catalog) = self.verify(route, value, langs).await? {
            return Ok(catalog);
        }
        let revoke = &route.spec.credentials.revoke;
        self.run_command(route, revoke, name, None, langs).await?;
        self.run_command(route, issue, name, Some(value), langs)
            .await?;
        if let Some(catalog) = self.verify(route, value, langs).await? {
            return Ok(catalog);
        }
        Err(unavailable(crate::l10n::text(
            langs,
            "plugin-settings-models-credential-rejected",
            &[
                ("provider", route.id.as_str().into()),
                ("name", name.into()),
            ],
        )))
    }

    /// 用凭据请求 `GET <openai>/models`：200 是认（带回模型列表），401 是不认，其他算网关出错。
    async fn verify(
        &self,
        route: &ModelsRoute,
        value: &str,
        langs: &Languages,
    ) -> Result<Option<serde_json::Value>, InstallError> {
        let path = route.spec.apis.openai.trim_end_matches('/');
        let url = format!("http://{}{path}/models", route.upstream);
        let (status, catalog) = self
            .http
            .request_json(
                Method::GET,
                &url,
                Some(value),
                RequestBody::Empty,
                VERIFY_TIMEOUT,
            )
            .await
            .map_err(|e| {
                unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-models-unreachable",
                    &[
                        ("provider", route.id.as_str().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        match status {
            StatusCode::OK => Ok(Some(catalog)),
            StatusCode::UNAUTHORIZED => Ok(None),
            other => Err(unavailable(crate::l10n::text(
                langs,
                "plugin-settings-models-list-error",
                &[
                    ("provider", route.id.as_str().into()),
                    ("status", format!("{other}").into()),
                ],
            ))),
        }
    }

    /// 默认路由探测用的凭据和模型列表：凭据是核心自己的（`<网关插件 id>/health`），独立于使用方的
    /// 限额与模型白名单，探测用量不记到 Bot 名下。值存在网关插件目录下，网关数据丢了重新签发。
    pub(super) async fn model_probe_catalog(
        &self,
        route: &ModelsRoute,
        langs: &Languages,
    ) -> Result<(String, serde_json::Value), InstallError> {
        let store = self.store.model_keys();
        let _guard = store.probe_lock.lock().await;
        let dir = store.dir.join("health");
        create_private_dir(&dir)?;
        let path = dir.join(format!("{}.json", route.id));
        let key = match read_json::<String>(&path)? {
            Some(key) => key,
            None => {
                let key = random_token();
                write_json(&path, &key)?;
                key
            }
        };
        let name = format!("{}/health", route.id);
        let catalog = self
            .issue_verified_catalog(route, &name, &key, langs)
            .await?;
        Ok((key, catalog))
    }
}
