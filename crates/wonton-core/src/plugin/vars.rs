use super::core_address;
use super::egress;
use super::interpolate::Vars;
use super::lan;
use super::manifest::{Manifest, Permission};
use super::render::InstallContext;
use super::secrets::secret_var;

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
        if let Some(email) = &owner.email {
            set("WONTON_OWNER_EMAIL", email.clone());
        }
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
    if let Some(url) = &ctx.mcp_url {
        set("WONTON_MCP_URL", url.clone());
    }
    if let Some(grant) = &ctx.lan {
        set(lan::ADDRESSES_VAR, grant.var());
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
        set("WONTON_MATRIX_OWNER", matrix.owner.clone());
        set("WONTON_MATRIX_HOME_ROOM", matrix.home_room.clone());
    }
    vars
}
