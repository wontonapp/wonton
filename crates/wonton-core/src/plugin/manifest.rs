//! 插件清单的服务端语义校验；类型和 serde 解析由 `wonton-protocol` 提供。

use super::languages::Languages;
pub use super::manifest_agent::{AgentLogin, AgentProtocol, AgentSpec};
pub use super::manifest_device::{DeviceClass, DeviceSlot, ManifestDevices};
pub use super::manifest_gpu_images::GpuImages;
pub(super) use super::manifest_mcp::DEFAULT_TOOL_TOTAL_SECONDS;
pub use super::manifest_mcp::{McpEndpoint, McpTransport};
pub use super::manifest_permission::PermissionExt;
pub use super::manifest_permission::{Access, Permission};
pub use super::manifest_remote::RemoteMcpSpec;
pub use super::manifest_static::{LinkSpec, StaticWebSpec};
use super::manifest_tabs;
pub use super::manifest_tabs::ManifestTab;
pub use super::manifest_widget::{MatrixWidget, WidgetEntry};
pub use super::requirements::{Arch, GpuNeed, Requirements};
pub use super::secret_spec::{
    CollectSecret, GenerateSecret, InputSecret, SecretEncoding, SecretSpec, ShowOwner,
};
pub use super::secret_spec::{MAX_SECRET_BYTES, is_secret_name};
use std::collections::BTreeMap;
pub(super) use wonton_protocol::manifest::is_label;
pub use wonton_protocol::manifest::{
    Accounts, BotProfile, BotServices, LocalizedTags, LocalizedText, MAX_ID_LEN, Manifest,
    ManifestError, OidcClient, OptionalPermission, PluginId, PluginKind, ProxyCredentials,
    RoleClaim, Runtime, Sso, TokenAuthMethod, WebUi,
};
use wonton_protocol::{ChatForm, PortProtocol};

/// 核心的权限计算和语义校验，不属于清单的线格式。
pub trait ManifestExt {
    fn gets_owner_identity(&self) -> bool;
    fn all_permissions(&self) -> impl Iterator<Item = &Permission>;
    fn optional_permission(&self, permission: &Permission) -> Option<&OptionalPermission>;
    fn effective_permissions(&self, enabled: &[Permission]) -> Vec<Permission>;
    fn validate(&self, langs: &Languages) -> Result<(), ManifestError>;
}

impl ManifestExt for Manifest {
    /// 容器拿不拿得到主人的身份变量。
    fn gets_owner_identity(&self) -> bool {
        self.permissions.contains(&Permission::OwnerIdentity)
    }

    /// 声明过的全部权限：必需的在前，可选的按权限名。校验按它遍历。
    fn all_permissions(&self) -> impl Iterator<Item = &Permission> {
        self.permissions
            .iter()
            .chain(self.optional_permissions.keys())
    }

    /// 一项权限是不是声明成可选、默认关。
    fn optional_permission(&self, permission: &Permission) -> Option<&OptionalPermission> {
        self.optional_permissions.get(permission)
    }

    /// 生效集：必需权限加上现在开着的可选项。
    fn effective_permissions(&self, enabled: &[Permission]) -> Vec<Permission> {
        self.permissions
            .iter()
            .chain(
                enabled
                    .iter()
                    .filter(|permission| self.optional_permissions.contains_key(*permission)),
            )
            .cloned()
            .collect()
    }

    /// 校验 serde 表达不了的规则。服务名是否存在由 [`super::package`] 对照 compose 检查。
    /// `langs` 只用于给按语言写的校验错误选语言，文字本身在各自模块里。
    fn validate(&self, langs: &Languages) -> Result<(), ManifestError> {
        if !is_semver(&self.version) {
            return Err(ManifestError::InvalidVersion(self.version.clone()));
        }
        require_locales("title", &self.title)?;
        require_locales("description", &self.description)?;
        super::store_info::validate(self, langs)?;
        super::manifest_agent::validate(self)?;
        validate_bots(self)?;
        super::privileges::validate_manifest(self, langs)?;
        super::privileges::validate_optional(self, langs)?;
        super::gpu::validate(self, langs)?;
        super::manifest_gpu_images::validate(self, langs)?;
        super::device::validate(self, langs)?;
        super::requirements::validate(self, langs)?;
        super::install_notes::validate_install_timeout(self)?;
        super::lan::validate(self, langs)?;
        super::manifest_clients::validate(self, langs)?;
        validate_tunnel(self)?;
        super::provides::validate(self, langs)?;
        super::capabilities::validate(self, langs)?;
        super::manifest_static::validate(self)?;
        super::manifest_widget::validate(self)?;
        super::manifest_remote::validate(self, langs)?;
        super::proxy_auth::validate(self, langs)?;
        super::proxy_auth::validate_mcp_headers(self, langs)?;
        if let Some(webui) = &self.webui {
            validate_webui(webui, self.runtime)?;
        }
        if let Some(mcp) = &self.mcp {
            super::manifest_mcp::validate(mcp)?;
        }
        manifest_tabs::validate(self.tabs.as_deref(), self.webui.as_ref())?;
        self.secrets
            .iter()
            .try_for_each(|(n, s)| super::secret_spec::validate_secret(n, s))
    }
}

fn validate_tunnel(manifest: &Manifest) -> Result<(), ManifestError> {
    let Some(tunnel) = &manifest.tunnel else {
        return Ok(());
    };
    let mcp = manifest.mcp.as_ref().map(|m| (&m.service, m.port));
    let mut seen = BTreeMap::new();
    for port in &tunnel.ports {
        if port.port < 1024 {
            return Err(ManifestError::Invalid(
                "tunnel.ports requires port >= 1024".to_owned(),
            ));
        }
        // 请求头和同号本机监听都不带 service，一个插件的端口/协议只能指向一个服务。
        if let Some(service) = seen.insert((port.port, port.protocol), &port.service) {
            return Err(ManifestError::Invalid(format!(
                "duplicate tunnel.ports endpoint {}: services {service} and {}",
                port.key(),
                port.service
            )));
        }
        // MCP is TCP; a UDP listener with the same number is a separate endpoint, as in lan.ports.
        if port.protocol == PortProtocol::Tcp && mcp == Some((&port.service, port.port)) {
            return Err(ManifestError::Invalid(
                "tunnel.ports cannot expose the MCP endpoint".to_owned(),
            ));
        }
    }
    Ok(())
}

/// `bot`、`bots` 和 `agent.protocol: matrix` 都以 Bot 身份收发消息，必须申请 `matrix.bot`。
fn validate_bots(manifest: &Manifest) -> Result<(), ManifestError> {
    super::bot_avatars::validate_manifest(manifest)?;
    if manifest
        .bot
        .as_ref()
        .is_some_and(|bot| bot.chat_layout == ChatForm::Other)
    {
        return Err(ManifestError::Invalid(
            "bot.chat_layout 只能是 bubbles 或 document".to_owned(),
        ));
    }
    let wants_bot = [
        ("bot", manifest.bot.is_some()),
        ("bots", manifest.bots.is_some()),
        (
            "agent.protocol: matrix",
            manifest.agent.as_ref().map(|a| a.protocol) == Some(AgentProtocol::Matrix),
        ),
    ];
    if let Some((field, _)) = wants_bot.iter().find(|(_, declared)| *declared)
        && !manifest.permissions.contains(&Permission::MatrixBot)
    {
        return Err(ManifestError::Invalid(format!(
            "声明了 {field} 就必须申请 matrix.bot 权限"
        )));
    }
    let Some(bots) = &manifest.bots else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    if bots.services.is_empty() || !bots.services.iter().all(|s| seen.insert(s)) {
        return Err(ManifestError::Invalid(
            "bots.services 至少一项，且不能重复".to_owned(),
        ));
    }
    // 局域网端口按服务开在宿主上：Bot 服务每个 Bot 一组容器，同一端口会互相冲突。
    if let Some(lan) = &manifest.lan {
        for port in &lan.ports {
            if bots.services.contains(&port.service) {
                return Err(ManifestError::Invalid(format!(
                    "network.lan 的服务 {} 是 Bot 服务：每个 Bot 一组容器，端口开不了，换全插件一份的服务",
                    port.service
                )));
            }
        }
    }
    Ok(())
}

fn validate_webui(webui: &WebUi, runtime: Runtime) -> Result<(), ManifestError> {
    if let Some(problem) = manifest_tabs::validate_path("webui.path", &webui.path) {
        return Err(ManifestError::Invalid(problem));
    }
    if let Some(health) = &webui.health
        && let Some(problem) = manifest_tabs::validate_path("webui.health", health)
    {
        return Err(ManifestError::Invalid(problem));
    }
    // 静态网页的 webui 只有展示字段，服务端字段由 manifest_static 按“用不上”报。
    if runtime == Runtime::StaticWeb {
        return Ok(());
    }
    if webui.service.is_empty() {
        return Err(ManifestError::Invalid("webui.service 必填".to_owned()));
    }
    if webui.port == 0 {
        return Err(ManifestError::Invalid(
            "webui.port 必填，且不能为 0".to_owned(),
        ));
    }
    let invalid = |message: &str| Err(ManifestError::Invalid(message.to_owned()));
    if webui.oidc.is_some() && webui.sso != Sso::Oidc {
        return invalid("webui.oidc 只能和 sso: oidc 一起使用");
    }
    if webui.proxy.is_some() && webui.sso != Sso::Proxy {
        return invalid("webui.proxy 只能和 sso: proxy 一起使用");
    }
    match (webui.sso, &webui.oidc, &webui.proxy) {
        (Sso::Oidc, Some(client), _) => {
            if client.redirect_paths.is_empty()
                || client.redirect_paths.iter().any(|p| !p.starts_with('/'))
            {
                return invalid("webui.oidc.redirect_paths 至少一项，且每项以 / 开头");
            }
            if client.post_logout_paths.iter().any(|p| !p.starts_with('/')) {
                return invalid("webui.oidc.post_logout_paths 每项以 / 开头");
            }
            super::manifest_oidc::validate(client)?;
            Ok(())
        }
        (Sso::Oidc, None, _) => invalid("webui.sso 为 oidc 时必须提供 webui.oidc.redirect_paths"),
        (Sso::Proxy, _, Some(proxy)) if proxy.headers.is_empty() && proxy.cookies.is_empty() => {
            invalid("webui.proxy 至少声明一个请求头或 Cookie")
        }
        (Sso::Proxy, _, None) => invalid("webui.sso 为 proxy 时必须提供 webui.proxy"),
        _ => Ok(()),
    }
}

pub(super) fn require_locales(
    field: &'static str,
    text: &LocalizedText,
) -> Result<(), ManifestError> {
    if text.is_empty() || text.values().any(|value| value.trim().is_empty()) {
        return Err(ManifestError::MissingLocale { field });
    }
    Ok(())
}

/// 语义化版本：`MAJOR.MINOR.PATCH`，可带 `-预发布` 和 `+构建` 后缀。
fn is_semver(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let numbers: Vec<&str> = core.split('.').collect();
    let suffix_ok = version[core.len()..]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    numbers.len() == 3
        && numbers.iter().all(|n| {
            !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
                && (n.len() == 1 || !n.starts_with('0'))
        })
        && suffix_ok
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod manifest_tests;
