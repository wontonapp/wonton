//! 第三方客户端入口开关的持久化和入口模型应用（ADR-0021、ADR-0090）。

use std::collections::BTreeSet;

use crate::clients::state::Kind;
use crate::entry_settings::EntrySettings;
use crate::gateway::Gateway;
use crate::plugin::clients_entry::SetupCheck;
use crate::plugin::manager::ClientPlugin;
use crate::plugin::manifest::PluginId;
use crate::plugin::store::InstalledRecord;
use wonton_protocol::{CertificateState, ClientAuthKind, ClientsFailure, EntryBlock};

pub(crate) struct ApplyError {
    pub failure: ClientsFailure,
    /// 语言包里的键（`clients-switch-` 开头），或只有服务器知道的细节。
    pub message: String,
    /// 键的变量；细节为空。
    args: Vec<(&'static str, String)>,
    /// 出错后的补充（回滚失败这类），整节拼在后面。
    notes: Vec<(&'static str, ApplyError)>,
}

impl ApplyError {
    /// 本模块自己的固定文案都是键。管理器不带语言，它给的细节只有中文，
    /// 所以套进 `clients-switch-failed`，细节仍看得懂。
    pub(crate) fn localized_message(&self, langs: &crate::plugin::languages::Languages) -> String {
        let args: Vec<(&str, wonton_l10n::Arg<'_>)> = self
            .args
            .iter()
            .map(|(name, value)| (*name, value.as_str().into()))
            .collect();
        let mut text = if self.message.starts_with("clients-switch-") {
            crate::l10n::text(langs, &self.message, &args)
        } else {
            crate::l10n::text(
                langs,
                "clients-switch-failed",
                &[("error", self.message.as_str().into())],
            )
        };
        for (key, cause) in &self.notes {
            let detail = cause.localized_message(langs);
            text.push(' ');
            text.push_str(&crate::l10n::text(
                langs,
                key,
                &[("error", detail.as_str().into())],
            ));
        }
        text
    }

    /// 在整句后面追加一节，类别不变。
    fn note(&mut self, key: &'static str, cause: ApplyError) {
        self.notes.push((key, cause));
    }
}

pub(crate) fn error(failure: ClientsFailure, message: impl Into<String>) -> ApplyError {
    ApplyError {
        failure,
        message: message.into(),
        args: Vec::new(),
        notes: Vec::new(),
    }
}

pub(crate) fn keyed(
    failure: ClientsFailure,
    key: &'static str,
    args: Vec<(&'static str, String)>,
) -> ApplyError {
    ApplyError {
        failure,
        message: key.to_owned(),
        args,
        notes: Vec::new(),
    }
}

/// 开关改完之后的样子：要回给 App 的阻断原因，和这次会不会让应用重启。
pub(crate) struct Applied {
    pub blocked: Option<EntryBlock>,
    pub restarts: bool,
}

pub(crate) async fn set_entry(
    gateway: &Gateway,
    id: &PluginId,
    kind: Kind,
    enabled: bool,
) -> Result<Applied, ApplyError> {
    let _serial = gateway.clients_apply.lock().await;
    let client = find_client(gateway, id)?;
    if enabled {
        check_can_open(gateway, &client, kind).await?;
    }

    let _record = gateway.plugins.clients_record_lock().await;
    // 验证可能会等待 setup 检查；重新确认插件仍然存在后再修改记录。
    find_client(gateway, id)?;
    let before = client.record.clone();
    let after = switched(&before, kind, enabled);
    // 规范对外地址变了就得重启用到它的服务；没变的不动（ADR-0089 第 5 节）。
    let value_changed =
        external_url_of(gateway, &before).await != external_url_of(gateway, &after).await;
    if enabled {
        apply_model(gateway, Some((id, kind))).await?;
        if let Err(message) = gateway.plugins.update_client_entry(id, kind, true) {
            let mut failure = error(ClientsFailure::PluginUnavailable, message);
            if let Err(rollback) = apply_model(gateway, None).await {
                failure.note("clients-switch-rollback-entry", rollback);
            }
            return Err(failure);
        }
    } else {
        let old = gateway
            .plugins
            .update_client_entry(id, kind, false)
            .map_err(|message| error(ClientsFailure::PluginUnavailable, message))?;
        if let Err(mut failure) = apply_model(gateway, None).await {
            if let Err(rollback) = gateway.plugins.restore_client_entry(&old) {
                failure.note(
                    "clients-switch-rollback-switch",
                    error(ClientsFailure::PluginUnavailable, rollback),
                );
            }
            return Err(failure);
        }
    }
    let mut restarts = false;
    if value_changed {
        match rebuild_external_url(gateway, &after).await {
            Ok(did) => restarts = did,
            Err(mut failure) => {
                // 重启失败：开关、入口模型和容器都退回原样，回不去的逐项写进详情。
                if let Err(rollback) = gateway.plugins.restore_client_entry(&before) {
                    failure.note(
                        "clients-switch-rollback-switch",
                        error(ClientsFailure::PluginUnavailable, rollback),
                    );
                } else {
                    if let Err(rollback) = apply_model(gateway, None).await {
                        failure.note("clients-switch-rollback-entry", rollback);
                    }
                    let _ = rebuild_external_url(gateway, &before).await;
                }
                return Err(failure);
            }
        }
    }
    // 开启只在配置应用成功后发布；关闭先撤销连接，失败回滚也不能复活旧连接。
    gateway.plugins.publish_client_entries();
    Ok(Applied {
        blocked: (kind == Kind::Public && enabled && !gateway.public_access_enabled())
            .then_some(EntryBlock::PublicAccessOff),
        restarts,
    })
}

fn switched(record: &InstalledRecord, kind: Kind, enabled: bool) -> InstalledRecord {
    let mut next = record.clone();
    match kind {
        Kind::Lan => next.clients_entries.lan = enabled,
        Kind::Public => next.clients_entries.public = enabled,
    }
    next
}

async fn external_url_of(gateway: &Gateway, record: &InstalledRecord) -> Option<String> {
    crate::clients::external_url::for_record(gateway, record).await
}

async fn rebuild_external_url(
    gateway: &Gateway,
    record: &InstalledRecord,
) -> Result<bool, ApplyError> {
    gateway
        .plugins
        .rebuild_external_url_users(record)
        .await
        .map_err(|cause| error(ClientsFailure::PluginUnavailable, cause.to_string()))
}

pub(crate) async fn block_reason(
    gateway: &Gateway,
    client: &ClientPlugin,
) -> Result<Option<EntryBlock>, ApplyError> {
    let entries = client.record.clients_entries;
    let credentials_pending = matches!(client.auth, ClientAuthKind::Credentials)
        && !gateway.plugins.credentials_ready(&client.id);
    if !(entries.lan || entries.public) {
        return Ok(credentials_pending.then_some(EntryBlock::CredentialsNotIssued));
    }
    let domain_ready = match &gateway.domain {
        Some(domain) => {
            let view = domain.view().await.map_err(|cause| {
                keyed(
                    ClientsFailure::PluginUnavailable,
                    "clients-switch-domain-read",
                    vec![("error", format!("{cause:#}"))],
                )
            })?;
            view.suffix.is_some() && certificate_ready(&view.certificate)
        }
        None => false,
    };
    if !domain_ready {
        return Ok(Some(EntryBlock::NoDomain));
    }
    if entries.lan
        && gateway
            .entry
            .as_ref()
            .is_none_or(|entry| !entry.lan().usable())
    {
        return Ok(Some(EntryBlock::LanUnavailable));
    }
    let http = gateway
        .plugins
        .route(&client.id.slug(), "/")
        .and_then(|route| route.clients)
        .ok_or_else(|| {
            keyed(
                ClientsFailure::PluginUnavailable,
                "clients-switch-no-route",
                vec![("plugin", client.id.to_string())],
            )
        })?;
    if http.http.auth.app.is_some() {
        match gateway
            .plugins
            .clients_setup_done(&client.id, &http, gateway.notices.as_ref())
            .await
        {
            SetupCheck::Pending => return Ok(Some(EntryBlock::SetupPending)),
            SetupCheck::Done => {}
            SetupCheck::Unavailable => {
                return Err(error(
                    ClientsFailure::PluginUnavailable,
                    "clients-switch-app-unavailable",
                ));
            }
            SetupCheck::Undetermined(cause) => {
                return Err(keyed(
                    ClientsFailure::PluginUnavailable,
                    "clients-switch-setup-check",
                    vec![("error", cause.to_string())],
                ));
            }
        }
    } else if credentials_pending {
        return Ok(Some(EntryBlock::CredentialsNotIssued));
    }
    if entries.public && !gateway.public_ingress_configured {
        return Ok(Some(EntryBlock::IngressNotConfigured));
    }
    if entries.public && !gateway.public_access_enabled() {
        return Ok(Some(EntryBlock::PublicAccessOff));
    }
    Ok(None)
}

/// 调用方持有 clients_apply 和 lan.pass；卸载前关闭入口并撤销现存连接。
pub(crate) async fn close_for_uninstall(
    gateway: &Gateway,
    id: &PluginId,
) -> Result<(), ApplyError> {
    let Some(client) = gateway
        .plugins
        .client_plugins()
        .map_err(|message| error(ClientsFailure::PluginUnavailable, message))?
        .into_iter()
        .find(|client| client.id == *id)
    else {
        // 记录读不出的客户端不会出现在列表里；它的路由可能还在 crt-list 上，照样重算一遍。
        if gateway
            .plugins
            .route(&id.slug(), "/")
            .and_then(|route| route.clients)
            .is_some()
        {
            apply_model(gateway, None).await?;
        }
        return Ok(());
    };
    if !client.record.clients_entries.lan && !client.record.clients_entries.public {
        return Ok(());
    }
    let old = gateway
        .plugins
        .update_client_entry(id, Kind::Lan, false)
        .map_err(|message| error(ClientsFailure::PluginUnavailable, message))?;
    let result = match gateway.plugins.update_client_entry(id, Kind::Public, false) {
        Ok(_) => apply_model(gateway, None).await,
        Err(message) => Err(error(ClientsFailure::PluginUnavailable, message)),
    };
    if let Err(mut failure) = result {
        if let Err(cause) = gateway.plugins.restore_client_entry(&old) {
            let rollback = error(ClientsFailure::PluginUnavailable, cause);
            failure.note("clients-switch-rollback-switch", rollback);
        }
        return Err(failure);
    }
    Ok(())
}

/// 对账和启动共用开关模型；不得依赖尚未恢复的内存插件表。
pub async fn reconcile(gateway: &Gateway) -> Result<(), String> {
    let Some(entry) = &gateway.entry else {
        return Ok(());
    };
    let _serial = gateway.clients_apply.lock().await;
    let _record = gateway.plugins.clients_record_lock().await;
    if EntrySettings::load(entry.install_dir())
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return entry
            .reconcile(None)
            .await
            .map_err(|error| error.to_string());
    }
    apply_model(gateway, None)
        .await
        .map_err(|error| error.localized_message(&crate::plugin::languages::Languages::default()))
}

fn find_client(gateway: &Gateway, id: &PluginId) -> Result<ClientPlugin, ApplyError> {
    gateway
        .plugins
        // 入口接口只控制 HTTP；TCP 开关由各自协议处理。
        .client_plugins()
        .map_err(|message| error(ClientsFailure::PluginUnavailable, message))?
        .into_iter()
        .find(|client| client.id == *id && client.http)
        .ok_or_else(|| {
            keyed(
                ClientsFailure::PluginUnavailable,
                "clients-switch-not-found",
                vec![("plugin", id.to_string())],
            )
        })
}

async fn check_can_open(
    gateway: &Gateway,
    client: &ClientPlugin,
    kind: Kind,
) -> Result<(), ApplyError> {
    let entry = gateway.entry.as_ref().ok_or_else(|| {
        error(
            ClientsFailure::CoreNotInContainer,
            "clients-switch-container",
        )
    })?;
    if !entry.core_in_container() {
        return Err(error(
            ClientsFailure::CoreNotInContainer,
            "clients-switch-container",
        ));
    }

    let domain = gateway
        .domain
        .as_ref()
        .ok_or_else(|| error(ClientsFailure::Conflict, "clients-switch-domain"))?;
    let view = domain.view().await.map_err(|cause| {
        keyed(
            ClientsFailure::PluginUnavailable,
            "clients-switch-domain-read",
            vec![("error", format!("{cause:#}"))],
        )
    })?;
    let suffix = match view.suffix {
        Some(suffix) if certificate_ready(&view.certificate) => suffix,
        _ => return Err(error(ClientsFailure::Conflict, "clients-switch-domain")),
    };

    if kind == Kind::Lan {
        super::lan_forward::check_open(gateway, entry, &suffix).await?;
    }

    let route = gateway
        .plugins
        .route(&client.id.slug(), "/")
        .and_then(|route| route.clients)
        .ok_or_else(|| {
            keyed(
                ClientsFailure::PluginUnavailable,
                "clients-switch-no-route",
                vec![("plugin", client.id.to_string())],
            )
        })?;
    if route.upstream.is_none() {
        return Err(keyed(
            ClientsFailure::PluginUnavailable,
            "clients-switch-service-down",
            vec![("plugin", client.id.to_string())],
        ));
    }
    if route.http.auth.app.is_some() {
        match gateway
            .plugins
            .clients_setup_done(&client.id, &route, gateway.notices.as_ref())
            .await
        {
            SetupCheck::Done => {}
            SetupCheck::Pending => {
                return Err(error(ClientsFailure::SetupPending, "clients-switch-setup"));
            }
            SetupCheck::Unavailable => {
                return Err(error(
                    ClientsFailure::PluginUnavailable,
                    "clients-switch-app-unavailable",
                ));
            }
            SetupCheck::Undetermined(cause) => {
                tracing::error!(plugin = %client.id, %cause, "检查客户端应用首次注册状态失败");
                return Err(keyed(
                    ClientsFailure::PluginUnavailable,
                    "clients-switch-setup-check",
                    vec![("error", cause.to_string())],
                ));
            }
        }
    } else if route.http.auth.credentials.is_some()
        && !gateway.plugins.credentials_ready(&client.id)
    {
        return Err(error(
            ClientsFailure::Conflict,
            "clients-switch-credentials",
        ));
    }

    if kind == Kind::Public && !gateway.public_ingress_configured {
        return Err(error(ClientsFailure::Conflict, "clients-switch-ingress"));
    }
    Ok(())
}

fn certificate_ready(certificate: &CertificateState) -> bool {
    matches!(
        certificate,
        CertificateState::Valid { .. } | CertificateState::Renewing { .. }
    )
}

async fn apply_model(
    gateway: &Gateway,
    opening: Option<(&PluginId, Kind)>,
) -> Result<(), ApplyError> {
    let Some(entry) = gateway.entry.as_ref() else {
        return Err(error(
            ClientsFailure::CoreNotInContainer,
            "clients-switch-container",
        ));
    };
    let settings = EntrySettings::load(entry.install_dir()).map_err(|cause| {
        keyed(
            ClientsFailure::IngressUnavailable,
            "clients-switch-settings-read",
            vec![("error", format!("{cause:#}"))],
        )
    })?;
    let Some(settings) = settings else {
        return Err(error(
            ClientsFailure::IngressUnavailable,
            "clients-switch-settings-missing",
        ));
    };
    let (public, lan) = labels(gateway, opening)?;
    entry
        .apply_clients_model(&settings, gateway.public_access_enabled(), public, lan)
        .await
        .map_err(|cause| {
            keyed(
                ClientsFailure::IngressUnavailable,
                "clients-switch-apply",
                vec![("error", cause.to_string())],
            )
        })
}

fn labels(
    gateway: &Gateway,
    opening: Option<(&PluginId, Kind)>,
) -> Result<(Vec<String>, Vec<String>), ApplyError> {
    let mut public = BTreeSet::new();
    let mut assigned = BTreeSet::new();
    let mut lan = BTreeSet::new();
    for file in gateway.plugins.store().load_all().map_err(|cause| {
        keyed(
            ClientsFailure::PluginUnavailable,
            "clients-switch-records-read",
            vec![("error", format!("{cause}"))],
        )
    })? {
        // 坏记录只少它自己的路由；不能让它挡住别的插件的开关和对账。打开的那条
        // 记录坏了的话，前面的 find_client 已经报过错。
        let mut record = match file.record {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(plugin = %file.id, %error, "插件记录读不了，重算入口时跳过");
                continue;
            }
        };
        if let Some((id, kind)) = opening
            && *id == file.id
        {
            match kind {
                Kind::Lan => record.clients_entries.lan = true,
                Kind::Public => record.clients_entries.public = true,
            }
        }
        if !record.clients_entries.public && !record.clients_entries.lan {
            continue;
        }
        let label = record.entry_label.ok_or_else(|| {
            keyed(
                ClientsFailure::PluginUnavailable,
                "clients-switch-no-label",
                vec![("plugin", file.id.to_string())],
            )
        })?;
        if !assigned.insert(label.clone()) {
            return Err(keyed(
                ClientsFailure::PluginUnavailable,
                "clients-switch-label-conflict",
                vec![("label", label.clone())],
            ));
        }
        if record.clients_entries.public {
            public.insert(label.clone());
        }
        if record.clients_entries.lan {
            lan.insert(label);
        }
    }
    Ok((public.into_iter().collect(), lan.into_iter().collect()))
}
