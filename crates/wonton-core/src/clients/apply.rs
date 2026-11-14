//! 第三方客户端入口开关的持久化和入口模型应用（ADR-0021、ADR-0090）。

use std::collections::BTreeSet;

use crate::clients::state::Kind;
use crate::entry_settings::EntrySettings;
use crate::gateway::Gateway;
use crate::plugin::clients_entry::SetupCheck;
use crate::plugin::manager::ClientPlugin;
use crate::plugin::manifest::PluginId;
use wonton_protocol::{CertificateState, ClientsFailure, EntryBlock};

pub(crate) struct ApplyError {
    pub failure: ClientsFailure,
    pub message: String,
}

impl ApplyError {
    pub(crate) fn localized_message(&self, langs: &crate::plugin::languages::Languages) -> String {
        if self.message.starts_with("clients-switch-") {
            crate::l10n::text(langs, &self.message, &[])
        } else {
            crate::l10n::text(
                langs,
                "clients-switch-failed",
                &[("error", self.message.clone().into())],
            )
        }
    }
}

fn error(failure: ClientsFailure, message: impl Into<String>) -> ApplyError {
    ApplyError {
        failure,
        message: message.into(),
    }
}

pub(crate) async fn set_entry(
    gateway: &Gateway,
    id: &PluginId,
    kind: Kind,
    enabled: bool,
) -> Result<Option<EntryBlock>, ApplyError> {
    let _serial = gateway.clients_apply.lock().await;
    let client = find_client(gateway, id)?;
    if enabled {
        check_can_open(gateway, &client, kind).await?;
    }

    let _record = gateway.plugins.clients_record_lock().await;
    // 验证可能会等待 setup 检查；重新确认插件仍然存在后再修改记录。
    find_client(gateway, id)?;
    if enabled {
        apply_model(gateway, Some((id, kind))).await?;
        if let Err(message) = gateway.plugins.update_client_entry(id, kind, true) {
            let mut failure = error(ClientsFailure::PluginUnavailable, message);
            if let Err(rollback) = apply_model(gateway, None).await {
                failure
                    .message
                    .push_str(&format!("；回滚入口配置失败：{}", rollback.message));
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
                failure
                    .message
                    .push_str(&format!("；回滚开关失败：{rollback}"));
            }
            return Err(failure);
        }
    }
    // 开启只在配置应用成功后发布；关闭先撤销连接，失败回滚也不能复活旧连接。
    gateway.plugins.publish_client_entries();
    Ok(
        (kind == Kind::Public && enabled && !gateway.public_access_enabled())
            .then_some(EntryBlock::PublicAccessOff),
    )
}

pub(crate) async fn block_reason(
    gateway: &Gateway,
    client: &ClientPlugin,
) -> Result<Option<EntryBlock>, ApplyError> {
    let entries = client.record.clients_entries;
    if !(entries.lan || entries.public) {
        return Ok(None);
    }
    let domain_ready = match &gateway.domain {
        Some(domain) => {
            let view = domain.view().await.map_err(|cause| {
                error(
                    ClientsFailure::PluginUnavailable,
                    format!("读取域名和证书状态失败：{cause:#}"),
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
            .is_none_or(|entry| !entry.lan_forward_available())
    {
        return Ok(Some(EntryBlock::LanUnavailable));
    }
    let http = gateway
        .plugins
        .route(&client.id.slug(), "/")
        .and_then(|route| route.clients)
        .ok_or_else(|| {
            error(
                ClientsFailure::PluginUnavailable,
                format!("客户端插件 {} 的入口不可用", client.id),
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
                    "应用没有运行，无法检查首次注册状态",
                ));
            }
            SetupCheck::Undetermined(cause) => {
                return Err(error(
                    ClientsFailure::PluginUnavailable,
                    format!("检查应用首次注册状态失败：{cause}"),
                ));
            }
        }
    } else if http.http.auth.credentials.is_some() {
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
            failure
                .message
                .push_str(&format!("；回滚开关失败：{cause}"));
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
        .map_err(|error| error.message)
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
            error(
                ClientsFailure::PluginUnavailable,
                format!("客户端插件 {id} 没有安装或没有声明入口"),
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
        error(
            ClientsFailure::PluginUnavailable,
            format!("读取域名和证书状态失败：{cause:#}"),
        )
    })?;
    if view.suffix.is_none() || !certificate_ready(&view.certificate) {
        return Err(error(ClientsFailure::Conflict, "clients-switch-domain"));
    }

    if kind == Kind::Lan && !entry.lan_forward_available() {
        return Err(error(ClientsFailure::LanUnavailable, "clients-switch-lan"));
    }

    let route = gateway
        .plugins
        .route(&client.id.slug(), "/")
        .and_then(|route| route.clients)
        .ok_or_else(|| {
            error(
                ClientsFailure::PluginUnavailable,
                format!("客户端插件 {} 没有可用的 HTTP 入口", client.id),
            )
        })?;
    if route.upstream.is_none() {
        return Err(error(
            ClientsFailure::PluginUnavailable,
            format!("客户端插件 {} 的服务没有运行", client.id),
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
                return Err(error(
                    ClientsFailure::PluginUnavailable,
                    format!("检查应用首次注册状态失败：{cause}"),
                ));
            }
        }
    } else if route.http.auth.credentials.is_some() {
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
            "core_not_in_container：核心没有入口容器控制，无法确认配置",
        ));
    };
    let settings = EntrySettings::load(entry.install_dir()).map_err(|cause| {
        error(
            ClientsFailure::IngressUnavailable,
            format!("读取入口设置失败：{cause:#}"),
        )
    })?;
    let Some(settings) = settings else {
        return Err(error(
            ClientsFailure::IngressUnavailable,
            "入口设置不存在，无法确认并应用入口模型",
        ));
    };
    let (public, lan) = labels(gateway, opening)?;
    entry
        .apply_clients_model(&settings, gateway.public_access_enabled(), public, lan)
        .await
        .map_err(|cause| {
            error(
                ClientsFailure::IngressUnavailable,
                format!("入口配置写入或重载失败：{cause}"),
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
        error(
            ClientsFailure::PluginUnavailable,
            format!("读取插件记录失败：{cause}"),
        )
    })? {
        let mut record = file.record.map_err(|cause| {
            error(
                ClientsFailure::PluginUnavailable,
                format!("读取插件记录失败：{cause}"),
            )
        })?;
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
            error(
                ClientsFailure::PluginUnavailable,
                format!("客户端插件 {} 没有入口标签", file.id),
            )
        })?;
        if !assigned.insert(label.clone()) {
            return Err(error(
                ClientsFailure::PluginUnavailable,
                format!("入口标签 {label} 冲突"),
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
