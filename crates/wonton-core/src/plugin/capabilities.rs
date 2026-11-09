//! 核心认识的能力表（ADR-0053）：清单的 `requires`/`wants` 和 `provides` 的键只能用表里的名字，
//! 或者按 `x-<作者>.<名字>` 格式自己定义的第三方能力。核心只按名字配对提供方和使用方。
//!
//! 转发约束：使用方经 `<名字>.wonton.internal` 的访问走模型网关同一条转发路径（WebSocket 升级、流式
//! 不缓冲、不限时，Agent 协议会用到），第 2 步实现转发时不得另起一套。

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};
use super::provides::Provides;
use crate::l10n::text;

/// 一项核心定义的能力：名字加接口约定。
pub struct Capability {
    /// 能力名：小写字母和下划线。清单 `requires`/`wants` 和 `provides` 的键里写的就是它。
    pub name: &'static str,
    /// 接口约定：使用方能期待提供方支持什么。
    pub interface: &'static str,
    /// 提供方要不要声明凭据命令（ADR-0053）。
    pub credentials: Credentials,
    /// 是不是高权限能力（ADR-0053 的“高权限能力”）：使用方连上它要主人确认。第三方能力不在这张表里，
    /// 按提供方算——提供方申请了 ADR-0036 的任一级高权限，或者它是 Agent 插件，就算高权限。
    pub high_privilege: bool,
}

/// 提供方要不要声明签发和吊销凭据的命令（ADR-0053）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Credentials {
    /// 不用凭据：提供方只认经核心转发来的请求，声明了反而是错的。
    None,
    /// 可以声明，声明了就给每个使用方注入 `WONTON_<名字>_KEY`。
    Optional,
    /// 必须声明，没有它使用方进不去。
    Required,
}

/// 模型网关（ADR-0008）：专用字段照旧在 `provides.models`，使用方申请 `models` 权限。
pub const MODELS: Capability = Capability {
    name: "models",
    interface: "OpenAI 兼容 API（provides.models.apis），凭据按使用方隔离",
    credentials: Credentials::Required,
    high_privilege: false,
};

/// 出网代理（ADR-0039）：没有使用方声明，申请了 `network.egress` 的插件都经它，不能写在 requires/wants 里。
/// 连接确认不适用（没有使用方声明），风险由 `egress.proxy` 高权限在安装时管。
pub const EGRESS_PROXY: Capability = Capability {
    name: "egress_proxy",
    interface: "HTTP 代理：出口设置在提供方（provides.egress_proxy）",
    credentials: Credentials::None,
    high_privilege: false,
};

/// 种子下载器：Flood、Prowlarr 这些使用方连它。
pub const TORRENT_CLIENT: Capability = Capability {
    name: "torrent_client",
    interface: "qBittorrent Web API v2（/api/v2/）",
    credentials: Credentials::None,
    high_privilege: false,
};

/// 媒体服务器：Seerr 这些使用方连它，凭据是每个使用方一把的 API key。
pub const JELLYFIN: Capability = Capability {
    name: "jellyfin",
    interface: "Jellyfin 的 HTTP API",
    credentials: Credentials::Required,
    high_privilege: false,
};

/// 全部核心定义的能力。加一项要改核心、发版：第三方插件作者不能自己发明不带 `x-` 的能力。
pub const ALL: &[Capability] = &[MODELS, EGRESS_PROXY, TORRENT_CLIENT, JELLYFIN];

pub fn lookup(name: &str) -> Option<&'static Capability> {
    ALL.iter().find(|capability| capability.name == name)
}

/// 能力名转成注入变量里的名字段：转大写，非字母数字换成 `_`（ADR-0053）。
pub fn variable_stem(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// 注入给使用方的地址变量，如 `WONTON_MODELS_URL`。
pub fn var_url(name: &str) -> String {
    format!("WONTON_{}_URL", variable_stem(name))
}

/// 注入给使用方的凭据变量，如 `WONTON_MODELS_KEY`（提供方声明了 `credentials` 时）。
pub fn var_key(name: &str) -> String {
    format!("WONTON_{}_KEY", variable_stem(name))
}

/// 使用方在插件网络里访问的内部别名，如 `models.wonton.internal`。
pub fn alias(name: &str) -> String {
    format!("{name}.wonton.internal")
}

/// 是不是第三方定义的能力名：`x-<作者>.<名字>`，作者和名字都是小写字母、数字、连字符（ADR-0053）。
pub fn is_third_party(name: &str) -> bool {
    let Some((author, capability)) = name
        .strip_prefix("x-")
        .and_then(|rest| rest.split_once('.'))
    else {
        return false;
    };
    is_label(author) && is_label(capability)
}

fn is_label(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// 能力名要么在表里，要么是合法的第三方名字；带 `x-` 但不合法的单说，别让主人误会要升级核心。
fn check_name(name: &str, langs: &Languages) -> Result<(), ManifestError> {
    if lookup(name).is_some() || is_third_party(name) {
        return Ok(());
    }
    if name.starts_with("x-") {
        return Err(ManifestError::Invalid(text(
            langs,
            "plugin-capability-third-party-format",
            &[("capability", name.into())],
        )));
    }
    Err(ManifestError::Invalid(text(
        langs,
        "plugin-capability-unknown",
        &[("capability", name.into())],
    )))
}

/// 这个插件是不是声明提供了这项能力。
pub fn is_provided(capability: &Capability, manifest: &Manifest) -> bool {
    let Some(provides) = &manifest.provides else {
        return false;
    };
    let (models, egress_proxy) = (MODELS.name, EGRESS_PROXY.name);
    match capability.name {
        x if x == models => provides.models.is_some(),
        x if x == egress_proxy => provides.egress_proxy.is_some(),
        _ => provides.capabilities.contains_key(capability.name),
    }
}

/// 清单的 `requires`/`wants` 和 `provides` 校验（ADR-0053）。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    for (field, names) in [("requires", &manifest.requires), ("wants", &manifest.wants)] {
        for name in names {
            check_name(name, langs)?;
            if name == EGRESS_PROXY.name {
                return Err(ManifestError::Invalid(text(
                    langs,
                    "plugin-capability-egress-no-consumer",
                    &[],
                )));
            }
        }
        if let Some(duplicate) = first_duplicate(names) {
            return Err(ManifestError::Invalid(text(
                langs,
                "plugin-capability-duplicate",
                &[("field", field.into()), ("capability", duplicate.into())],
            )));
        }
    }
    if let Some(name) = manifest
        .requires
        .iter()
        .find(|name| manifest.wants.contains(name))
    {
        return Err(ManifestError::Invalid(text(
            langs,
            "plugin-capability-in-both-lists",
            &[("capability", name.as_str().into())],
        )));
    }
    if manifest.provides.is_some() && !manifest.requires.is_empty() {
        return Err(ManifestError::Invalid(text(
            langs,
            "plugin-capability-provider-requires",
            &[],
        )));
    }
    if let Some(provides) = &manifest.provides {
        validate_provides(provides, langs)?;
    }
    Ok(())
}

fn validate_provides(provides: &Provides, langs: &Languages) -> Result<(), ManifestError> {
    for (name, service) in &provides.capabilities {
        check_name(name, langs)?;
        if service.port == 0 {
            return Err(ManifestError::Invalid(text(
                langs,
                "plugin-capability-port-zero",
                &[("capability", name.as_str().into())],
            )));
        }
        let required = matches!(
            lookup(name).map(|capability| capability.credentials),
            Some(Credentials::Required)
        );
        match (&service.credentials, required) {
            (None, true) => {
                return Err(ManifestError::Invalid(text(
                    langs,
                    "plugin-capability-credentials-required",
                    &[("capability", name.as_str().into())],
                )));
            }
            (Some(_), false)
                if lookup(name).is_some_and(|c| c.credentials == Credentials::None) =>
            {
                return Err(ManifestError::Invalid(text(
                    langs,
                    "plugin-capability-credentials-forbidden",
                    &[("capability", name.as_str().into())],
                )));
            }
            _ => {}
        }
        if let Some(credentials) = &service.credentials {
            for (field, argv) in [
                ("issue", &credentials.issue),
                ("revoke", &credentials.revoke),
            ] {
                if !argv
                    .iter()
                    .any(|arg| arg.contains(super::provides::NAME_PLACEHOLDER))
                {
                    return Err(ManifestError::Invalid(text(
                        langs,
                        "plugin-capability-credentials-argv",
                        &[
                            ("capability", name.as_str().into()),
                            ("field", field.into()),
                            ("placeholder", super::provides::NAME_PLACEHOLDER.into()),
                        ],
                    )));
                }
            }
        }
    }
    Ok(())
}

fn first_duplicate(names: &[String]) -> Option<&str> {
    names.iter().enumerate().find_map(|(i, name)| {
        names[..i]
            .iter()
            .any(|prior| prior == name)
            .then_some(name.as_str())
    })
}

/// 第 2 步接上行为之前的拦截：清单声明了按能力依赖（`requires`/`wants`），或者提供了能力表里的
/// 新能力（`models`、`egress_proxy` 的专用字段照旧能用），照常装上但依赖一样都不兑现——不一起装、
/// 不转发、没有 `<能力>.wonton.internal`，是伪装成功，所以按 `unsupported` 拒绝。
/// **第 2 步接上行为时删掉这个函数和它的调用。**
pub fn unsupported_dependencies(
    manifest: &Manifest,
    langs: &Languages,
) -> Result<(), super::install_error::InstallError> {
    let new_provides = manifest
        .provides
        .as_ref()
        .is_some_and(|provides| !provides.capabilities.is_empty());
    if manifest.requires.is_empty() && manifest.wants.is_empty() && !new_provides {
        return Ok(());
    }
    Err(super::install_error::InstallError::Unsupported(
        crate::l10n::text(langs, "plugin-install-capability-deps-unsupported", &[]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_names_follow_the_rule() {
        assert_eq!(variable_stem("models"), "MODELS");
        assert_eq!(var_url("models"), "WONTON_MODELS_URL");
        assert_eq!(var_key("models"), "WONTON_MODELS_KEY");
        assert_eq!(alias("models"), "models.wonton.internal");
        assert_eq!(var_url("x-acme.ledger"), "WONTON_X_ACME_LEDGER_URL");
        assert_eq!(var_key("x-acme.ledger"), "WONTON_X_ACME_LEDGER_KEY");
        assert_eq!(alias("x-acme.ledger"), "x-acme.ledger.wonton.internal");
    }

    /// 契约：能力表的别名和网关转发认得的主机是同一套——能力的访问走模型网关同一条转发路径
    /// （WebSocket 升级、流式不缓冲、不限时），第 2 步接 `<能力>.wonton.internal` 时不得另起一套。
    #[test]
    fn aliases_route_through_the_same_gateway_path() {
        assert_eq!(alias(MODELS.name), crate::config::PLUGIN_MODELS_HOST);
        for capability in ALL {
            if capability.name == EGRESS_PROXY.name {
                continue; // egress_proxy 没有使用方声明，出网插件拿的是 WONTON_PROXY_URL。
            }
            assert!(
                alias(capability.name).ends_with(".wonton.internal"),
                "{}",
                alias(capability.name)
            );
        }
    }

    #[test]
    fn third_party_names_are_recognized() {
        assert!(is_third_party("x-acme.ledger"));
        assert!(is_third_party("x-acme.ledger-v2"));
        assert!(!is_third_party("models"));
        assert!(!is_third_party("x-acme"));
        assert!(!is_third_party("x-.ledger"));
        assert!(!is_third_party("x-acme."));
        assert!(!is_third_party("x-ACME.ledger"));
        assert!(!is_third_party("x-acme.LEDGER"));
        assert!(!is_third_party("x-acme.led ger"));
    }

    fn manifest(requires: &[&str], wants: &[&str], provides: bool) -> Manifest {
        // provides 的专用字段（models）的合法性由 provides::validate 管，这里用通用条目当提供方。
        serde_json::from_value(serde_json::json!({
            "spec": "0.6",
            "id": "app.example",
            "version": "1.0.0",
            "kind": "app",
            "title": {"en": "Example"},
            "description": {"en": "Example plugin"},
            "permissions": [],
            "requires": requires,
            "wants": wants,
            "provides": if provides {
                serde_json::json!({"torrent_client": {"service": "qbittorrent", "port": 8080}})
            } else {
                serde_json::Value::Null
            },
        }))
        .unwrap()
    }

    #[test]
    fn known_capability_names_pass() {
        validate(&manifest(&["models"], &[], false), &Languages::default()).unwrap();
        validate(&manifest(&[], &["models"], false), &Languages::default()).unwrap();
        validate(&manifest(&[], &[], false), &Languages::default()).unwrap();
    }

    #[test]
    fn third_party_capability_names_pass() {
        validate(
            &manifest(&["x-acme.ledger"], &[], false),
            &Languages::default(),
        )
        .unwrap();
        validate(
            &manifest(&[], &["x-acme.ledger"], false),
            &Languages::default(),
        )
        .unwrap();
    }

    #[test]
    fn unknown_names_are_refused_with_an_upgrade_hint() {
        let e = validate(&manifest(&["search"], &[], false), &Languages::default()).unwrap_err();
        assert!(e.to_string().contains("search"));
    }

    #[test]
    fn malformed_third_party_names_get_their_own_error() {
        let e = validate(&manifest(&["x-acme"], &[], false), &Languages::default()).unwrap_err();
        assert!(e.to_string().contains("x-acme"));
    }

    #[test]
    fn a_capability_cannot_be_both_required_and_wanted() {
        let e = validate(
            &manifest(&["models"], &["models"], false),
            &Languages::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("models"));
    }

    #[test]
    fn providers_cannot_require() {
        let e = validate(&manifest(&["models"], &[], true), &Languages::default()).unwrap_err();
        // 文字按语言协商，断言只看行为：提供能力的插件不能 requires。
        drop(e);
    }

    #[test]
    fn egress_proxy_has_no_consumer_declaration() {
        let e = validate(
            &manifest(&["egress_proxy"], &[], false),
            &Languages::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("egress_proxy"));
        let e = validate(
            &manifest(&[], &["egress_proxy"], false),
            &Languages::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("egress_proxy"));
    }

    #[test]
    fn duplicates_are_refused() {
        let e = validate(
            &manifest(&["models", "models"], &[], false),
            &Languages::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("models"));
    }

    fn manifest_with_provides(provides: serde_json::Value) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "spec": "0.6",
            "id": "app.example",
            "version": "1.0.0",
            "kind": "app",
            "title": {"en": "Example"},
            "description": {"en": "Example plugin"},
            "permissions": [],
            "provides": provides,
        }))
        .unwrap()
    }

    #[test]
    fn third_party_providers_pass() {
        let manifest = manifest_with_provides(serde_json::json!({
            "x-acme.ledger": {
                "service": "ledger",
                "port": 8080,
                "credentials": {
                    "issue": ["issue", "{name}"],
                    "revoke": ["revoke", "{name}"],
                },
            },
        }));
        validate(&manifest, &Languages::default()).unwrap();
    }

    #[test]
    fn jellyfin_providers_must_declare_credentials() {
        let manifest = manifest_with_provides(
            serde_json::json!({"jellyfin": {"service": "jellyfin", "port": 8096}}),
        );
        let e = validate(&manifest, &Languages::default()).unwrap_err();
        assert!(e.to_string().contains("jellyfin"));
        assert!(e.to_string().contains("credentials"));
    }

    #[test]
    fn torrent_client_providers_cannot_declare_credentials() {
        let manifest = manifest_with_provides(serde_json::json!({
            "torrent_client": {
                "service": "qbittorrent",
                "port": 8080,
                "credentials": {"issue": ["i", "{name}"], "revoke": ["r", "{name}"]},
            },
        }));
        let e = validate(&manifest, &Languages::default()).unwrap_err();
        assert!(e.to_string().contains("torrent_client"));
        assert!(e.to_string().contains("credentials"));

        let manifest = manifest_with_provides(
            serde_json::json!({"torrent_client": {"service": "q", "port": 8080}}),
        );
        validate(&manifest, &Languages::default()).unwrap();
    }

    #[test]
    fn unknown_provided_capabilities_are_refused() {
        let manifest = manifest_with_provides(
            serde_json::json!({"search": {"service": "searxng", "port": 8080}}),
        );
        let e = validate(&manifest, &Languages::default()).unwrap_err();
        assert!(e.to_string().contains("search"));
    }

    #[test]
    fn provider_ports_and_credentials_are_checked() {
        let manifest = manifest_with_provides(
            serde_json::json!({"x-acme.ledger": {"service": "l", "port": 0}}),
        );
        let e = validate(&manifest, &Languages::default()).unwrap_err();
        assert!(e.to_string().contains("x-acme.ledger"));

        let manifest = manifest_with_provides(serde_json::json!({
            "x-acme.ledger": {
                "service": "ledger",
                "port": 8080,
                "credentials": {"issue": ["issue"], "revoke": ["revoke"]},
            },
        }));
        let e = validate(&manifest, &Languages::default()).unwrap_err();
        assert!(
            e.to_string()
                .contains("provides.x-acme.ledger.credentials.issue")
        );
    }
}
