//! 装卸接口的角色门槛（ADR-0088）：`Allow` 放行，`AskAdmin` 发一张卡片给所有管理员并等回答，
//! `Deny` 与不认识的取值按最严处理。等卡片和执行都在后台任务里：设备的连接断了，管理员批准后
//! 照样执行（ADR-0016 的做法）；拒绝回 403 `permission_required{InstallApps}`，过期回 408
//! `confirmation_expired`。
//!
//! 详情和商店的 `installable`/`install_gate`、开放范围（`audience`）也在这里按人投影。

use std::sync::Arc;

use axum::Extension;
use axum::response::{IntoResponse, Response};
use hyper::header::HeaderValue;
use hyper::{HeaderMap, StatusCode};
use tokio::sync::oneshot;
use wonton_protocol::{
    Answer, CardChoice, CardLine, Gate, InstallPreview, PERMISSION_REQUIRED, PermissionItem,
    PermissionRequired,
};

use super::api::{acted_by, error_response, install_error};
use super::pages::ERROR_HEADER;
use super::{Gateway, Person, TunnelPeer};
use crate::l10n;
use crate::mcp::{Draft, Refusal};
use crate::plugin::languages::Languages;
use crate::plugin::{InstallError, PluginId, UpgradeTo};

pub(crate) enum GateOutcome<R> {
    /// 卡片在后台等管理员；断开连接不影响它。等回来的值已经是
    /// [`Outcome`]：调用方以 `GateOutcome<Outcome<…>>` 收结果。
    Wait(oneshot::Receiver<R>),
    Stop(Response),
}

pub(crate) enum Outcome<R> {
    Done(R),
    Rejected,
    Expired,
    Unavailable(String),
}

/// 已经在 [`GateOutcome::Wait`] 里的结果怎么收：等到的按结果返回，拒绝按 403、过期按 408，文字按请求的语言。
#[expect(
    clippy::result_large_err,
    reason = "网关的错误响应都是同一个大小，这条函数不做拆分"
)]
pub(crate) async fn settle<R>(
    gate: GateOutcome<Outcome<R>>,
    action: &str,
    langs: &Languages,
) -> Result<R, Response> {
    match gate {
        GateOutcome::Wait(waited) => match waited.await {
            Ok(Outcome::Done(result)) => Ok(result),
            Ok(Outcome::Rejected) => Err(error_response(
                StatusCode::FORBIDDEN,
                PERMISSION_REQUIRED,
                &l10n::text(
                    langs,
                    "gateway-install-rejected",
                    &[("action", action.into())],
                ),
            )),
            Ok(Outcome::Unavailable(message)) => Err(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "confirmation_unavailable",
                &message,
            )),
            _ => Err(error_response(
                StatusCode::REQUEST_TIMEOUT,
                "confirmation_expired",
                &l10n::text(
                    langs,
                    "gateway-install-expired",
                    &[("action", action.into())],
                ),
            )),
        },
        GateOutcome::Stop(response) => Err(response),
    }
}

fn permission_required() -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        axum::Json(PermissionRequired {
            missing: PermissionItem::InstallApps,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(ERROR_HEADER, HeaderValue::from_static(PERMISSION_REQUIRED));
    response
}

/// 不能装的人在读请求体之前就被挡下（ADR-0088）。
pub(crate) fn deny_check(gateway: &Gateway, person: &Person) -> Option<Response> {
    match gateway.permissions(person).install_apps {
        Gate::Allow | Gate::AskAdmin => None,
        Gate::Deny | Gate::Other => Some(permission_required()),
    }
}

/// `AskAdmin` 的等待：卡片在后台等管理员，拒绝和过期按 [`Outcome`] 送回；批准了执行 `run`。
fn ask_admin<R, F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    draft: Draft,
    run: F,
) -> GateOutcome<Outcome<R>>
where
    R: Send + 'static,
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send,
{
    let bus = gateway
        .mcp
        .as_ref()
        .expect("调用方已判过有总线")
        .bus
        .clone();
    let identity = acted_by(gateway, peer.as_ref(), headers);
    let gateway = Arc::clone(gateway);
    let (sent, waited) = oneshot::channel();
    tokio::spawn(async move {
        let outcome = match bus.ask_plugin_admin(identity, draft).await {
            Ok(_ticket) => Outcome::Done(run(gateway).await),
            Err(Refusal::Rejected) => Outcome::Rejected,
            Err(Refusal::Unavailable(message)) => Outcome::Unavailable(message),
            Err(_) => Outcome::Expired,
        };
        let _ = sent.send(outcome);
    });
    GateOutcome::Wait(waited)
}

fn choices(langs: &Languages) -> Vec<CardChoice> {
    vec![
        CardChoice {
            answer: Answer::Approve,
            label: l10n::text(langs, "chat-choice-install", &[]),
            choice_id: None,
        },
        CardChoice {
            answer: Answer::Reject,
            label: l10n::text(langs, "chat-choice-deny-install", &[]),
            choice_id: None,
        },
    ]
}

/// 按来源安装的门槛：卡片内容按预览写，批准后执行 `run`。
pub(crate) async fn install_source<R, F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
    request: &wonton_protocol::InstallRequest,
    run: F,
) -> GateOutcome<Outcome<R>>
where
    R: Send + 'static,
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send,
{
    match gateway.permissions(person).install_apps {
        Gate::Allow => GateOutcome::Wait(ready(Outcome::Done(run(gateway.clone()).await))),
        wonton_protocol::Gate::AskAdmin => {
            let langs = gateway.plugins.owner_languages();
            match gateway
                .plugins
                .preview_source(&request.source, &langs)
                .await
            {
                Ok(preview) => {
                    let draft = preview_draft(&preview, &langs);
                    ask_admin(gateway, peer, headers, draft, run)
                }
                Err(e) => GateOutcome::Stop(install_error(&e)),
            }
        }
        _ => GateOutcome::Stop(permission_required()),
    }
}

/// 把已经算出的结果包成“立刻就绪”的等待，让调用方一条路收结果。
fn ready<R>(result: Outcome<R>) -> oneshot::Receiver<Outcome<R>> {
    let (sent, waited) = oneshot::channel();
    let _ = sent.send(result);
    waited
}

/// 上传插件包安装的门槛：卡片按包的预览写，批准后执行 `run`。
pub(crate) async fn install_upload<R, F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
    package: &crate::plugin::PluginPackage,
    run: F,
) -> GateOutcome<Outcome<R>>
where
    R: Send + 'static,
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send,
{
    match gateway.permissions(person).install_apps {
        Gate::Allow => GateOutcome::Wait(ready(Outcome::Done(run(gateway.clone()).await))),
        wonton_protocol::Gate::AskAdmin => {
            let langs = gateway.plugins.owner_languages();
            match gateway
                .plugins
                .preview_upload(package.clone(), &langs)
                .await
            {
                Ok(preview) => {
                    let draft = preview_draft(&preview, &langs);
                    ask_admin(gateway, peer, headers, draft, run)
                }
                Err(e) => GateOutcome::Stop(install_error(&e)),
            }
        }
        _ => GateOutcome::Stop(permission_required()),
    }
}

/// 升级的门槛：卡片内容按升级预览写，批准后执行 `run`。
pub(crate) async fn upgrade<R, F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
    id: &PluginId,
    to: UpgradeTo,
    run: F,
) -> GateOutcome<Outcome<R>>
where
    R: Send + 'static,
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send,
{
    match gateway.permissions(person).install_apps {
        Gate::Allow => GateOutcome::Wait(ready(Outcome::Done(run(gateway.clone()).await))),
        wonton_protocol::Gate::AskAdmin => {
            let langs = gateway.plugins.owner_languages();
            match gateway.plugins.preview_upgrade(id, to, &langs).await {
                Ok(preview) => {
                    let draft = preview_draft(&preview, &langs);
                    ask_admin(gateway, peer, headers, draft, run)
                }
                Err(e) => GateOutcome::Stop(install_error(&e)),
            }
        }
        _ => GateOutcome::Stop(permission_required()),
    }
}

/// 卸载的门槛：卡片按已装应用写，批准后执行 `run`。
pub(crate) async fn uninstall<R, F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
    id: &PluginId,
    run: F,
) -> GateOutcome<Outcome<R>>
where
    R: Send + 'static,
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send,
{
    match gateway.permissions(person).install_apps {
        Gate::Allow => GateOutcome::Wait(ready(Outcome::Done(run(gateway.clone()).await))),
        wonton_protocol::Gate::AskAdmin => {
            let langs = gateway.plugins.owner_languages();
            let Some(installed) = gateway.plugins.installed_plugin(id, &langs) else {
                return GateOutcome::Stop(install_error(&InstallError::NotInstalled(
                    id.to_string(),
                )));
            };
            let draft = Draft {
                title: l10n::text(
                    &langs,
                    "chat-card-uninstall-title",
                    &[
                        ("requester", person.username.clone().into()),
                        ("title", installed.title.clone().into()),
                    ],
                ),
                lines: Vec::new(),
                choices: vec![
                    CardChoice {
                        answer: Answer::Approve,
                        label: l10n::text(&langs, "chat-choice-uninstall", &[]),
                        choice_id: None,
                    },
                    CardChoice {
                        answer: Answer::Reject,
                        label: l10n::text(&langs, "chat-choice-deny", &[]),
                        choice_id: None,
                    },
                ],
                scope: None,
                plugin_id: Some(id.to_string()),
                secret_inputs: Vec::new(),
                settings: None,
                category: None,
                permissions: Vec::new(),
                devices: Vec::new(),
            };
            ask_admin(gateway, peer, headers, draft, run)
        }
        _ => GateOutcome::Stop(permission_required()),
    }
}

fn preview_draft(preview: &InstallPreview, langs: &Languages) -> Draft {
    let mut lines = vec![CardLine {
        text: l10n::text(
            langs,
            "chat-card-version-official",
            &[("version", preview.version.as_str().into())],
        ),
        emphasis: false,
    }];
    lines.extend(preview.permissions.iter().map(|line| CardLine {
        text: format!("· {}", line.text),
        emphasis: line.risk.is_some(),
    }));
    lines.extend(preview.notices.iter().cloned());
    Draft {
        title: preview.title.clone(),
        lines,
        choices: choices(langs),
        scope: None,
        plugin_id: Some(preview.id.clone()),
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    }
}

/// 详情页按人投影：`scope` 只给能装卸的人，`installable` 按 `AskAdmin` 收紧，
/// 开放范围只有主人能改、别人只拿到风险类别；分享只给有“分享”权限的人。
pub(crate) fn project_detail(
    gateway: &Gateway,
    person: &Person,
    id: &str,
    detail: &mut wonton_protocol::PluginDetail,
) {
    let permissions = gateway.permissions(person);
    let gate = permissions.install_apps;
    detail.install_gate = gate;
    detail.installable = gate == Gate::Allow;
    if gate != Gate::Allow {
        detail.scope = None;
    }
    let installed = detail.installed.is_some();
    if let Some(view) = detail.audience.as_mut() {
        super::plugin_audience::populate(gateway, person, id, installed, view);
        if !person.is_owner() {
            view.choices.clear();
        }
    }
    if !permissions.allows(wonton_protocol::PermissionItem::ShareApps) {
        detail.shares = None;
    }
}

pub(crate) fn project_preview(gateway: &Gateway, person: &Person, preview: &mut InstallPreview) {
    preview.install_gate = gateway.permissions(person).install_apps;
    if !person.is_owner() {
        preview.audience = preview.audience.take().map(|mut view| {
            view.choices.clear();
            view
        });
    }
}

pub(crate) fn project_store(
    gateway: &Gateway,
    person: &Person,
    item: &mut wonton_protocol::StoreItem,
) {
    let gate = gateway.permissions(person).install_apps;
    item.install_gate = gate;
    item.installable = gate == Gate::Allow;
}
