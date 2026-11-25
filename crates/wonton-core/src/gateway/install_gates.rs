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
    Answer, CardChoice, Gate, InstallPreview, PERMISSION_REQUIRED, PermissionItem,
    PermissionRequired,
};

use super::api::{acted_by, error_response, install_error};
use super::install_gate_cards::{admin_cards, filled_lines, source_admin_cards};
use super::install_gate_checks::{self, Installed, RecordState, UpgradeAsk};
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

/// 403 `permission_required`，带缺的那一项：不能装的人缺“装、删、升级应用”，要管理员批的人
/// 想改权限缺的是主人（ADR-0057）。
fn permission_required(missing: PermissionItem) -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        axum::Json(PermissionRequired { missing }),
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
        Gate::Deny | Gate::Other => Some(permission_required(PermissionItem::InstallApps)),
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
            Ok(_) => Outcome::Done(run(gateway).await),
            Err(Refusal::Rejected) => Outcome::Rejected,
            Err(Refusal::Unavailable(message)) => Outcome::Unavailable(message),
            Err(_) => Outcome::Expired,
        };
        let _ = sent.send(outcome);
    });
    GateOutcome::Wait(waited)
}

fn ask_admin_nvme<F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    (first, second, success, upgrade_id): (Draft, Draft, String, Option<String>),
    run: F,
) -> GateOutcome<Outcome<Response>>
where
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = Response> + Send,
{
    let bus = gateway
        .mcp
        .as_ref()
        .expect("调用方已判过有总线")
        .bus
        .clone();
    let identity = acted_by(gateway, peer.as_ref(), headers);
    let langs = gateway.plugins.owner_languages();
    let gateway = Arc::clone(gateway);
    let (sent, waited) = oneshot::channel();
    tokio::spawn(async move {
        let outcome = match bus.ask_plugin_admin_twice(identity, first, second).await {
            Ok((first, second)) => {
                let response = run(gateway).await;
                let state = if response.status().is_success() {
                    wonton_protocol::ConfirmationState::Succeeded { message: success }
                } else {
                    let status = response.status().to_string();
                    let kind = response
                        .headers()
                        .get(ERROR_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("http_error");
                    let message = if let Some(id) = &upgrade_id {
                        l10n::text(
                            &langs,
                            "chat-update-failed",
                            &[
                                ("id", id.as_str().into()),
                                ("kind", kind.into()),
                                ("error", status.as_str().into()),
                            ],
                        )
                    } else {
                        l10n::text(
                            &langs,
                            "chat-install-failed",
                            &[("kind", kind.into()), ("error", status.as_str().into())],
                        )
                    };
                    wonton_protocol::ConfirmationState::Failed { message }
                };
                bus.confirmations().finish(first, state.clone()).await;
                bus.confirmations().finish(second, state).await;
                Outcome::Done(response)
            }
            Err(Refusal::Rejected) => Outcome::Rejected,
            Err(Refusal::Unavailable(message)) => Outcome::Unavailable(message),
            Err(_) => Outcome::Expired,
        };
        let _ = sent.send(outcome);
    });
    GateOutcome::Wait(waited)
}

/// 发卡：选了 NVMe 或有最高级权限时两张都要批（ADR-0036），否则一张。
fn send_cards<F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    (first, second, success, upgrade_id): (Draft, Option<Draft>, String, Option<String>),
    run: F,
) -> GateOutcome<Outcome<Response>>
where
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = Response> + Send,
{
    match second {
        Some(second) => ask_admin_nvme(
            gateway,
            peer,
            headers,
            (first, second, success, upgrade_id),
            run,
        ),
        None => ask_admin(gateway, peer, headers, first, run),
    }
}

/// 按来源安装的门槛：卡片内容按预览写，批准后执行 `run`。要管理员批时先按 [`install_gate_checks`]
/// 核对请求；执行按请求里同一个摘要取包，和卡片上的不一致就停。
pub(crate) async fn install_source<F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
    request: &wonton_protocol::InstallRequest,
    run: F,
) -> GateOutcome<Outcome<Response>>
where
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = Response> + Send,
{
    match gateway.permissions(person).install_apps {
        Gate::Allow => GateOutcome::Wait(ready(Outcome::Done(run(gateway.clone()).await))),
        Gate::AskAdmin => {
            if install_gate_checks::install_changes_permissions(request) {
                return GateOutcome::Stop(permission_required(PermissionItem::Owner));
            }
            let langs = gateway.plugins.owner_languages();
            let cards = async {
                let preview = gateway
                    .plugins
                    .preview_source(&request.source, &langs)
                    .await?;
                install_gate_checks::matches_install_preview(
                    &preview,
                    &request.digest,
                    &request.plan,
                    &langs,
                )?;
                let (first, second) =
                    source_admin_cards(gateway, request, &preview, &langs).await?;
                Ok::<_, InstallError>((first, second, preview.title))
            };
            match cards.await {
                Ok((first, second, title)) => {
                    let success = l10n::text(
                        &langs,
                        "chat-install-done",
                        &[("title", title.as_str().into())],
                    );
                    send_cards(gateway, peer, headers, (first, second, success, None), run)
                }
                Err(e) => GateOutcome::Stop(install_error(&e)),
            }
        }
        _ => GateOutcome::Stop(permission_required(PermissionItem::InstallApps)),
    }
}

/// 把已经算出的结果包成“立刻就绪”的等待，让调用方一条路收结果。
fn ready<R>(result: Outcome<R>) -> oneshot::Receiver<Outcome<R>> {
    let (sent, waited) = oneshot::channel();
    let _ = sent.send(result);
    waited
}

/// 上传插件包安装的门槛：卡片按包的预览写，批准后执行 `run`（装的是同一份包）。`changes` 是查询串
/// 要打开可选权限或局域网开关，要管理员批的人不能这样装（ADR-0057）。
pub(crate) async fn install_upload<F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
    (package, selected, changes): (&crate::plugin::PluginPackage, &[String], bool),
    run: F,
) -> GateOutcome<Outcome<Response>>
where
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = Response> + Send,
{
    match gateway.permissions(person).install_apps {
        Gate::Allow => GateOutcome::Wait(ready(Outcome::Done(run(gateway.clone()).await))),
        Gate::AskAdmin => {
            if changes {
                return GateOutcome::Stop(permission_required(PermissionItem::Owner));
            }
            let langs = gateway.plugins.owner_languages();
            let cards = async {
                let preview = gateway
                    .plugins
                    .preview_upload(package.clone(), &langs)
                    .await?;
                install_gate_checks::installable(&preview)?;
                let (mut first, second) = admin_cards(&preview, selected, (false, &[]), &langs)?;
                let secrets = super::api_switch::secrets(headers)?;
                let settings = super::api_switch::settings(headers)?;
                first
                    .lines
                    .extend(filled_lines(&preview, secrets.keys(), &settings, &langs));
                Ok::<_, InstallError>((first, second, preview.title))
            };
            match cards.await {
                Ok((first, second, title)) => {
                    let success = l10n::text(
                        &langs,
                        "chat-install-done",
                        &[("title", title.as_str().into())],
                    );
                    send_cards(gateway, peer, headers, (first, second, success, None), run)
                }
                Err(e) => GateOutcome::Stop(install_error(&e)),
            }
        }
        _ => GateOutcome::Stop(permission_required(PermissionItem::InstallApps)),
    }
}

/// 升级的门槛：卡片内容按升级预览写，批准后执行 `run`。预览按请求带的可选项全集算生效集，
/// 要管理员批时它必须和装着的一样（[`install_gate_checks::upgrade_changes_permissions`]）。
pub(crate) async fn upgrade<F, Fut>(
    gateway: &Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
    id: &PluginId,
    ask: UpgradeAsk,
    run: F,
) -> GateOutcome<Outcome<Response>>
where
    F: FnOnce(Arc<Gateway>) -> Fut + Send + 'static,
    Fut: Future<Output = Response> + Send,
{
    match gateway.permissions(person).install_apps {
        Gate::Allow => GateOutcome::Wait(ready(Outcome::Done(run(gateway.clone()).await))),
        Gate::AskAdmin => {
            let langs = gateway.plugins.owner_languages();
            match upgrade_cards(gateway, id, ask, &langs).await {
                Ok((first, second, version, state)) => {
                    let success = l10n::text(
                        &langs,
                        "chat-update-done",
                        &[("version", version.as_str().into())],
                    );
                    let cards = (first, second, success, Some(id.to_string()));
                    let id = id.clone();
                    let checked = move |gateway: Arc<Gateway>| async move {
                        let langs = gateway.plugins.owner_languages();
                        match state.still(&gateway.plugins, &id, &langs) {
                            Ok(()) => run(gateway).await,
                            Err(e) => install_error(&e),
                        }
                    };
                    send_cards(gateway, peer, headers, cards, checked)
                }
                Err(response) => GateOutcome::Stop(response),
            }
        }
        _ => GateOutcome::Stop(permission_required(PermissionItem::InstallApps)),
    }
}

/// 升级要发的卡：先核对摘要、能不能装、改没改权限，再按预览和记录里的 NVMe 写。另给发卡时的
/// [`RecordState`]，执行前再比一次。
#[expect(
    clippy::result_large_err,
    reason = "网关的错误响应都是同一个大小，这条函数不做拆分"
)]
async fn upgrade_cards(
    gateway: &Gateway,
    id: &PluginId,
    mut ask: UpgradeAsk,
    langs: &Languages,
) -> Result<(Draft, Option<Draft>, String, RecordState), Response> {
    let failed = |error: InstallError| install_error(&error);
    let (record, state) = RecordState::read(&gateway.plugins, id, langs).map_err(failed)?;
    let installed = Installed::of(&record, langs).map_err(failed)?;
    let to = std::mem::replace(&mut ask.to, UpgradeTo::Offered);
    let preview = gateway
        .plugins
        .preview_upgrade(id, to, ask.enabled.as_deref(), langs)
        .await
        .map_err(failed)?;
    if ask
        .digest
        .as_deref()
        .is_some_and(|digest| digest != preview.digest)
    {
        return Err(failed(install_gate_checks::changed_upgrade(
            &preview, id, langs,
        )));
    }
    // 先看装不装得了：设备列表读不到时是 `device_unavailable`，不是换了设备要主人。
    install_gate_checks::installable(&preview).map_err(failed)?;
    if install_gate_checks::upgrade_changes_permissions(&preview, &installed, &ask) {
        return Err(permission_required(PermissionItem::Owner));
    }
    let kept = install_gate_checks::kept_nvme(&preview, &record);
    let (mut first, second) =
        admin_cards(&preview, &ask.devices, (true, &kept), langs).map_err(failed)?;
    first
        .lines
        .extend(filled_lines(&preview, &ask.secrets, &ask.settings, langs));
    Ok((first, second, preview.version, state))
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
        _ => GateOutcome::Stop(permission_required(PermissionItem::InstallApps)),
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
