//! 各视图共用的文字拼装和协议枚举映射。

use wonton_protocol::{CardLine, PluginAuthor, PluginKind as Kind, PluginTrust};

use super::super::github_source::{Origin, Trust};
use super::super::languages::Languages;
use super::super::manifest::{Manifest, Permission, PluginKind, Sso};
use super::super::views_permissions::permission_text;
use crate::l10n;

/// 不是系统插件时：按来源装的看来源，上传的是本地插件。
pub(in crate::plugin) fn trust(origin: Option<&Origin>) -> PluginTrust {
    match origin.map(|o| o.trust) {
        Some(Trust::Official) => PluginTrust::Official,
        Some(Trust::Community) => PluginTrust::Community,
        None => PluginTrust::Local,
    }
}

pub(crate) fn cross_origin_isolation_notice(
    manifest: &Manifest,
    langs: &Languages,
) -> Option<CardLine> {
    if !manifest
        .webui
        .as_ref()
        .is_some_and(|webui| webui.cross_origin_isolated)
    {
        return None;
    }
    Some(CardLine {
        text: l10n::text(langs, "plugin-cross-origin-isolation", &[]),
        emphasis: true,
    })
}

/// 清单里的 `setup`：主人装之前就得看到的一步，醒目显示。
pub(super) fn setup_notice(
    m: &super::super::manifest::Manifest,
    langs: &Languages,
) -> Option<CardLine> {
    m.setup.as_ref().map(|setup| CardLine {
        text: langs.pick_owned(setup),
        emphasis: true,
    })
}

/// 装好之后第一眼的说明：作者写的“装好后要做的一步”，以及用自己登录的网页、会露出身份的插件。
pub(super) fn opening_notices(m: &Manifest, langs: &Languages) -> Vec<CardLine> {
    let mut notices = Vec::new();
    if let Some(setup) = setup_notice(m, langs) {
        notices.push(setup);
    }
    if m.webui.as_ref().is_some_and(|w| w.sso != Sso::None) {
        notices.push(line(l10n::text(langs, "install-webui-sso", &[])));
    }
    // 写了权限的，已经在权限列表里了。
    if super::super::views_permissions::reveals_owner_identity(m)
        && !m.permissions.contains(&Permission::OwnerIdentity)
    {
        notices.push(line(permission_text(&Permission::OwnerIdentity, langs)));
    }
    notices
}

pub(in crate::plugin) fn line(text: String) -> CardLine {
    CardLine {
        text,
        emphasis: false,
    }
}

pub(in crate::plugin) fn author(manifest: &Manifest) -> Option<PluginAuthor> {
    manifest.author.as_ref().map(|a| PluginAuthor {
        name: a.name.clone(),
        url: a.url.clone(),
    })
}

pub(in crate::plugin) fn kind(kind: PluginKind) -> Kind {
    match kind {
        PluginKind::App => Kind::App,
        PluginKind::Agent => Kind::Agent,
        PluginKind::Channel => Kind::Channel,
        PluginKind::Model => Kind::Model,
        PluginKind::System => Kind::System,
    }
}
