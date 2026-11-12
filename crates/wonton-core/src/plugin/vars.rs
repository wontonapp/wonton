use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::core_address;
use super::egress;
use super::interpolate::Vars;
use super::lan;
use super::manifest::{Manifest, Permission};
use super::names;
use super::render::InstallContext;
use super::secrets::secret_var;

/// 注入给 OIDC 客户端的配置。`Debug` 不打印客户端密钥。
#[derive(Clone, PartialEq, Eq)]
pub struct OidcVars {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
}

impl std::fmt::Debug for OidcVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcVars")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// 注入给插件的主人身份（[`Manifest::gets_owner_identity`]），和 OIDC 声明、代登录的 `{user}`、`{email}` 同值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerVars {
    pub name: String,
    pub email: String,
}

/// 注入给 Bot 的 Matrix 身份（`matrix.bot`）。`Debug` 不打印访问令牌。
#[derive(Clone, PartialEq, Eq)]
pub struct MatrixVars {
    pub homeserver: String,
    pub user_id: String,
    pub access_token: String,
    /// `WONTON_MATRIX_ALLOWED_USERS`：逗号分隔的 Matrix 用户 ID，Bot 只听这些人的话（ADR-0045）。
    pub allowed_users: String,
    pub home_room: String,
}

impl std::fmt::Debug for MatrixVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatrixVars")
            .field("homeserver", &self.homeserver)
            .field("user_id", &self.user_id)
            .field("allowed_users", &self.allowed_users)
            .field("home_room", &self.home_room)
            .finish_non_exhaustive()
    }
}

/// 注入给使用方的模型网关配置（`models`）。记进使用方的凭据文件（见 [`super::models`]），重建容器时
/// 网关不在也注入同一套值。`Debug` 不打印凭据。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVars {
    pub base_url: String,
    pub anthropic_url: Option<String>,
    pub gemini_url: Option<String>,
    /// 插件级的凭据（凭据名是插件 `id`），全插件一份的服务用。
    pub plugin_key: Option<String>,
    /// 每个 Bot 的凭据（凭据名 `<id>/<Bot 名>`，键是 Bot 名），声明了 `bots` 时 Bot 服务用。
    /// 较早的核心没记这一项：缺省按空表读，整份记录还能读出来。
    #[serde(default)]
    pub bot_keys: BTreeMap<String, String>,
    pub default_model: String,
}

impl std::fmt::Debug for ModelVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelVars")
            .field("base_url", &self.base_url)
            .field("anthropic_url", &self.anthropic_url)
            .field("gemini_url", &self.gemini_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

pub fn share_var(name: &str) -> String {
    format!(
        "WONTON_SHARE_{}",
        name.to_ascii_uppercase().replace('-', "_")
    )
}

/// 核心为插件提供的插值变量。
pub fn injected_vars(manifest: &Manifest, ctx: &InstallContext) -> Vars {
    let mut vars = Vars::new();
    let mut set = |k: &str, v: String| {
        vars.insert(k.to_owned(), v);
    };
    set("WONTON_APP_ID", manifest.id.to_string());
    set("WONTON_APP_DATA", ctx.app_data.display().to_string());
    set("WONTON_SERVICE_DOMAIN", names::service_domain(&ctx.slug));
    // 取不到时是空串而不是未定义：插件写了 `${WONTON_HOST_NAME}` 也装得上，见 host_check。
    set("WONTON_HOST_NAME", ctx.host_name.clone());
    set("TZ", ctx.tz.clone());
    set("PUID", ctx.puid.to_string());
    set("PGID", ctx.pgid.to_string());
    for (name, path) in &ctx.shares {
        set(&share_var(name), path.display().to_string());
    }
    if let Some(url) = &ctx.app_url {
        set("WONTON_APP_URL", url.clone());
    }
    for (name, value) in &ctx.secrets {
        set(&secret_var(name), value.clone());
    }
    for (name, value) in super::settings::value::vars(&ctx.settings) {
        set(&name, value);
    }
    if let Some(oidc) = &ctx.oidc {
        set("WONTON_OIDC_ISSUER", oidc.issuer.clone());
        set("WONTON_OIDC_CLIENT_ID", oidc.client_id.clone());
        set("WONTON_OIDC_CLIENT_SECRET", oidc.client_secret.clone());
        set("WONTON_OIDC_AUTH_URL", oidc.auth_url.clone());
        set("WONTON_OIDC_TOKEN_URL", oidc.token_url.clone());
        set("WONTON_OIDC_USERINFO_URL", oidc.userinfo_url.clone());
    }
    if let Some(owner) = &ctx.owner {
        set("WONTON_OWNER_NAME", owner.name.clone());
        set("WONTON_OWNER_EMAIL", owner.email.clone());
    }
    if let Some(models) = &ctx.models {
        set("WONTON_MODEL_BASE_URL", models.base_url.clone());
        if let Some(url) = &models.anthropic_url {
            set("WONTON_MODEL_ANTHROPIC_BASE_URL", url.clone());
        }
        if let Some(url) = &models.gemini_url {
            set("WONTON_MODEL_GEMINI_BASE_URL", url.clone());
        }
        if let Some(key) = &models.plugin_key {
            set("WONTON_MODEL_API_KEY", key.clone());
        }
        set("WONTON_MODEL_DEFAULT", models.default_model.clone());
    }
    for (name, value) in &ctx.capabilities {
        set(name, value.clone());
    }
    if let Some(url) = &ctx.mcp_url {
        set("WONTON_MCP_URL", url.clone());
    }
    if let Some(url) = &ctx.notify_url {
        set("WONTON_NOTIFY_URL", url.clone());
    }
    if manifest.permissions.contains(&Permission::Notify) {
        set(
            "WONTON_SMTP_HOST",
            crate::config::PLUGIN_SMTP_HOST.to_owned(),
        );
        set("WONTON_SMTP_PORT", "25".to_owned());
        set(
            "WONTON_MAIL_FROM",
            format!("{}@plugins.wonton.internal", manifest.id),
        );
    }
    if let Some(grant) = &ctx.lan {
        set(lan::ADDRESSES_VAR, grant.var());
    }
    // 可选权限开着时给所有服务一个 1，关着时不定义（ADR-0057）。
    for permission in &ctx.granted {
        if let Some(var) = super::privileges::permission_var(permission) {
            set(var, "1".to_owned());
        }
    }
    if let Some(address) = &ctx.core_address {
        set(core_address::VAR, address.clone());
    }
    if manifest.permissions.contains(&Permission::NetworkEgress) {
        let url = ctx.egress.as_ref().map_or("", |_| egress::PROXY_URL);
        set(egress::URL_VAR, url.to_owned());
    }
    if let Some(matrix) = &ctx.matrix {
        set("WONTON_MATRIX_HOMESERVER", matrix.homeserver.clone());
        set("WONTON_MATRIX_USER_ID", matrix.user_id.clone());
        set("WONTON_MATRIX_ACCESS_TOKEN", matrix.access_token.clone());
        set("WONTON_MATRIX_ALLOWED_USERS", matrix.allowed_users.clone());
        set("WONTON_MATRIX_HOME_ROOM", matrix.home_room.clone());
    }
    vars
}
