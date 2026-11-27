//! `provides.<名字>.browse`（ADR-0113）：客户端核心经插件网页的代理直接浏览提供方的库，网关在转发上代加
//! 当前这个人的身份头，客户端不拿上游账号密码。所以它只能写在 `media_source` 上，插件还要有 `sso: proxy` 的网页。

use wonton_protocol::manifest::Sso;
use wonton_protocol::media::MEDIA_CAPABILITY;

use super::manifest::{Manifest, ManifestError};

pub(super) fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    let Some(provides) = &manifest.provides else {
        return Ok(());
    };
    for (name, service) in &provides.capabilities {
        if service.browse.is_none() {
            continue;
        }
        if name != MEDIA_CAPABILITY {
            return Err(ManifestError::Invalid(format!(
                "provides.{name}.browse 只能写在 {MEDIA_CAPABILITY} 上"
            )));
        }
        if manifest
            .webui
            .as_ref()
            .is_none_or(|webui| webui.sso != Sso::Proxy)
        {
            return Err(ManifestError::Invalid(format!(
                "provides.{name}.browse 要求插件有 webui 且 sso: proxy：客户端经网页的代理浏览，靠网关代加的身份头认人"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(capability: &str, sso: Option<&str>) -> Manifest {
        let mut value = serde_json::json!({
            "spec": "0.17",
            "id": "app.example",
            "version": "1.0.0",
            "kind": "app",
            "title": {"en": "Example"},
            "description": {"en": "Example plugin"},
            "permissions": [],
            "provides": {capability: {
                "service": "sidecar",
                "port": 8090,
                "operations": ["playback"],
                "kinds": ["audio"],
                "credentials": {"issue": ["i", "{name}"], "revoke": ["r", "{name}"]},
                "browse": "opensubsonic",
            }},
        });
        if let Some(sso) = sso {
            value["webui"] = serde_json::json!({"service": "web", "port": 4533, "sso": sso});
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn browse_needs_media_source_and_a_proxy_signed_web_page() {
        assert!(validate(&manifest(MEDIA_CAPABILITY, Some("proxy"))).is_ok());
        for (capability, sso) in [
            ("x-acme.music", Some("proxy")),
            (MEDIA_CAPABILITY, None),
            (MEDIA_CAPABILITY, Some("oidc")),
            (MEDIA_CAPABILITY, Some("none")),
        ] {
            let e = validate(&manifest(capability, sso)).unwrap_err();
            assert!(
                e.to_string().contains(".browse"),
                "{capability} {sso:?}: {e}"
            );
        }
    }
}
