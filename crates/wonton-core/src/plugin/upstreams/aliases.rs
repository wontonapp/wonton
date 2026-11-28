//! 插件在插件网络里能用的核心别名，以及判断插件是否走 SSO。

use super::super::manifest::{Manifest, Permission, Sso};
use crate::config::{
    PLUGIN_AUTH_HOST, PLUGIN_MATRIX_HOST, PLUGIN_MCP_HOST, PLUGIN_MODELS_HOST, PLUGIN_NOTIFY_HOST,
    PublicUrls,
};

pub(in crate::plugin) fn uses_sso(manifest: &Manifest) -> bool {
    manifest.webui.as_ref().is_some_and(|w| w.sso == Sso::Oidc)
}

/// 核心在插件网络里的别名：插件经它们访问核心代为转发的服务。SSO 插件经 `auth.wonton.internal` 访问身份提供方，
/// 有网页的插件经自己的公网地址访问自己（ADR-0063），Bot 经 `matrix.wonton.internal` 访问 Matrix 客户端接口，
/// 模型网关的使用方经 `models.wonton.internal`，Agent 经 `mcp.wonton.internal` 访问 MCP 总线。
pub(in crate::plugin) fn core_aliases(public: &PublicUrls, manifest: &Manifest) -> Vec<String> {
    let mut aliases = Vec::new();
    if uses_sso(manifest) {
        aliases.push(PLUGIN_AUTH_HOST.to_owned());
    }
    if manifest.webui.is_some() {
        aliases.push(public.plugin_host(&manifest.id.slug()));
    }
    let granted = |permission: Permission| manifest.permissions.contains(&permission);
    if granted(Permission::MatrixBot) {
        aliases.push(PLUGIN_MATRIX_HOST.to_owned());
    }
    if super::super::models::uses_models(manifest) {
        aliases.push(PLUGIN_MODELS_HOST.to_owned());
    }
    if granted(Permission::Tools) {
        aliases.push(PLUGIN_MCP_HOST.to_owned());
    }
    if granted(Permission::PluginsHealth) {
        aliases.push(crate::config::PLUGIN_HEALTH_HOST.to_owned());
    }
    if granted(Permission::Notify) {
        aliases.push(PLUGIN_NOTIFY_HOST.to_owned());
    }
    for capability in manifest.requires.iter().chain(&manifest.wants) {
        if capability == super::super::capabilities::MODELS.name
            || super::super::capabilities::has_no_consumer(capability)
            || (super::super::capabilities::lookup(capability).is_none()
                && !super::super::capabilities::is_third_party(capability))
        {
            continue;
        }
        let alias = super::super::capabilities::alias(capability);
        if !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    aliases
}

#[cfg(test)]
mod tests {
    use super::super::super::package::PluginPackage;
    use super::super::*;
    use crate::config::{PLUGIN_HEALTH_HOST, PLUGIN_MODELS_HOST, PublicUrls};
    use crate::plugin::languages::Languages;

    const MANIFEST: &str = "name: t
services:
  web:
    image: example/t
x-wonton:
  spec: '0.4'
  id: app.example.t
  version: 0.1.0
  kind: app
  title: {zh: 测试, en: Test}
  description: {zh: 测试, en: Test}
  permissions: [models, tools, plugins.health, matrix.bot]
";
    const SSO_MANIFEST: &str = "name: t
services:
  web:
    image: example/t
x-wonton:
  spec: '0.4'
  id: app.example.t
  version: 0.1.0
  kind: app
  title: {zh: 测试, en: Test}
  description: {zh: 测试, en: Test}
  webui:
    service: web
    port: 3000
    sso: oidc
    oidc:
      redirect_paths: [/oidc/callback]
  permissions: []
";
    fn public() -> PublicUrls {
        PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.example".into(),
            port: 8080,
        }
    }
    #[test]
    fn health_alias_is_only_for_plugins_that_asked_for_it() {
        let manifest = PluginPackage::parse(MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        let aliases = core_aliases(&public(), &manifest);
        assert!(aliases.contains(&PLUGIN_HEALTH_HOST.to_owned()));
        assert!(aliases.contains(&PLUGIN_MODELS_HOST.to_owned()));
        let mut without = PluginPackage::parse(MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        without
            .permissions
            .retain(|p| *p != Permission::PluginsHealth);
        assert!(!core_aliases(&public(), &without).contains(&PLUGIN_HEALTH_HOST.to_owned()));
    }
    #[test]
    fn sso_plugins_reach_the_provider_by_its_internal_name_and_their_own_host() {
        let manifest = PluginPackage::parse(SSO_MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        let aliases = core_aliases(&public(), &manifest);
        assert!(aliases.contains(&"auth.wonton.internal".to_owned()));
        assert!(!aliases.contains(&"auth.wonton.example".to_owned()));
        assert!(aliases.contains(&"app-example-t.wonton.example".to_owned()));
    }
    #[test]
    fn plugins_without_a_webui_get_no_alias_to_their_own_address() {
        let manifest = PluginPackage::parse(MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        assert!(
            !core_aliases(&public(), &manifest)
                .contains(&"app-example-t.wonton.example".to_owned())
        );
    }

    #[test]
    fn dependency_aliases_only_include_known_or_valid_third_party_capabilities() {
        let mut manifest = PluginPackage::parse(MANIFEST, &Languages::from_header(None))
            .unwrap()
            .manifest;
        manifest.permissions.clear();
        manifest.requires = vec!["torrent_client".into(), "x-acme.ledger".into()];
        manifest.wants = vec![
            "jellyfin".into(),
            "torrent_client".into(),
            "models".into(),
            "egress_proxy".into(),
            "unknown".into(),
            "x-acme.invalid_name".into(),
        ];
        assert_eq!(
            core_aliases(&public(), &manifest),
            vec![
                "models.wonton.internal",
                "torrent_client.wonton.internal",
                "x-acme.ledger.wonton.internal",
                "jellyfin.wonton.internal",
            ],
        );
    }
}
