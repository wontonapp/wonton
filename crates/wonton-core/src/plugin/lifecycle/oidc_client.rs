//! Compose OIDC callbacks are resolved against the exact application's origin.

use crate::oidc::ClientSpec;
use crate::plugin::manifest::Manifest;
use crate::plugin::render::InstallContext;

pub(in crate::plugin) fn client_spec(
    manifest: &Manifest,
    ctx: &InstallContext,
    secret: &str,
) -> ClientSpec {
    client_spec_at(manifest, ctx.app_url.as_deref().unwrap_or_default(), secret)
}
pub(in crate::plugin) fn client_spec_at(
    manifest: &Manifest,
    base: &str,
    secret: &str,
) -> ClientSpec {
    let webui = manifest.webui.as_ref();
    let oidc = webui.and_then(|w| w.oidc.as_ref());
    ClientSpec {
        id: manifest.id.to_string(),
        secret: secret.to_owned(),
        name: manifest
            .title
            .get("en")
            .or_else(|| manifest.title.values().next())
            .cloned()
            .unwrap_or_else(|| manifest.id.to_string()),
        redirect_uris: oidc
            .map(|o| {
                o.redirect_paths
                    .iter()
                    .map(|p| format!("{base}{p}"))
                    .collect()
            })
            .unwrap_or_default(),
        post_logout_redirect_uris: webui
            .map(|webui| {
                // Hydra 按整串匹配，根地址有无末尾斜杠都要登记。
                let mut paths = vec!["", "/", webui.path.as_str()];
                if let Some(oidc) = oidc {
                    paths.extend(oidc.post_logout_paths.iter().map(String::as_str));
                }
                paths.sort_unstable();
                paths.dedup();
                paths
                    .into_iter()
                    .map(|path| format!("{base}{path}"))
                    .collect()
            })
            .unwrap_or_default(),
        token_endpoint_auth_method: oidc
            .map(|o| o.token_endpoint_auth_method.as_str())
            .unwrap_or("client_secret_basic"),
    }
}
