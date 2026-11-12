//! 确认卡片执行时和执行完的设置与结果：把主人填的设置值变成核心值，升级跑完写结果卡片。

use std::sync::Arc;

use serde_json::json;
use wonton_protocol::{PluginWriter, SecretValues};

use super::cards;
use super::{Bus, Outcome};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::Permission;
use crate::plugin::store::EnabledPermission;
use crate::plugin::{PluginId, PluginPackage, PluginView};

/// 安装/升级卡片带上主人已填的设置值时，换成核心能存和注入的值；没填的用默认值。
pub(super) fn card_settings(
    package: &PluginPackage,
    given: &wonton_protocol::SettingValues,
    langs: &Languages,
) -> Result<crate::plugin::settings::Values, crate::plugin::InstallError> {
    match crate::plugin::settings::load(package, langs)? {
        Some(schema) => crate::plugin::settings::value::initial(&schema, given, langs)
            .map_err(|fields| crate::plugin::settings::value_error(fields, langs)),
        None if given.is_empty() => Ok(Default::default()),
        None => Err(crate::plugin::settings::value_error(
            given
                .keys()
                .map(|name| wonton_protocol::SettingsFieldError {
                    name: name.clone(),
                    kind: wonton_protocol::SettingsFieldProblemKind::UnknownField,
                    message: l10n::text(langs, "plugin-settings-value-unknown-field", &[]),
                })
                .collect(),
            langs,
        )),
    }
}

/// 更新执行完的结果：卡片上的话按主人的语言写，给 Agent 的还是结构化的 JSON。
/// `writer` 是接手时记下的归属（#817）；没记下来时下次更新还会再问一次主人，两边都要如实说。
#[allow(clippy::too_many_arguments)] // 卡片执行要用的几个值，再包一层也是逐个填。
pub(super) async fn upgrade_ran(
    bus: Arc<Bus>,
    id: PluginId,
    version: String,
    package: PluginPackage,
    granted: Vec<Permission>,
    enabled: Vec<EnabledPermission>,
    lan_webui: bool,
    secrets: SecretValues,
    settings: wonton_protocol::SettingValues,
    writer: Option<PluginWriter>,
) -> Outcome {
    let langs = bus.plugins.owner_languages();
    let filled = bus.plugins.store().secrets().load(&id).ok().flatten();
    if let Err(e) =
        bus.plugins
            .store_input_values(&package.manifest, &secrets, filled.as_ref(), &langs)
    {
        return Outcome::failed(
            e.kind(),
            l10n::text(
                &langs,
                "chat-install-failed",
                &[("kind", e.kind().into()), ("error", e.to_string().into())],
            ),
        );
    }
    match bus
        .plugins
        .upgrade_upload(&id, package, granted, enabled, lan_webui, &settings, &langs)
        .await
    {
        Ok(()) => {
            let note = match writer.map(|writer| bus.plugins.set_writer(&id, Some(writer))) {
                None | Some(Ok(())) => None,
                Some(Err(e)) => {
                    tracing::error!(plugin = %id, error = %e, "记插件的归属失败");
                    Some(l10n::text(
                        &langs,
                        "chat-install-note-writer",
                        &[("error", e.to_string().into())],
                    ))
                }
            };
            let url = bus
                .plugins
                .list()
                .into_iter()
                .find(|p| p.id == id)
                .and_then(|p| p.url);
            let message = match &url {
                Some(url) => l10n::text(
                    &langs,
                    "chat-update-done-url",
                    &[
                        ("version", version.as_str().into()),
                        ("url", url.as_str().into()),
                    ],
                ),
                None => l10n::text(
                    &langs,
                    "chat-update-done",
                    &[("version", version.as_str().into())],
                ),
            };
            let message = match note {
                Some(note) => l10n::text(
                    &langs,
                    "chat-outcome-with-note",
                    &[("message", message.into()), ("note", note.into())],
                ),
                None => message,
            };
            Outcome::ok(
                json!({"updated": id, "version": version, "url": url}).to_string(),
                message,
            )
        }
        Err(e) => Outcome::failed(
            e.kind(),
            l10n::text(
                &langs,
                "chat-update-failed",
                &[("kind", e.kind().into()), ("error", e.to_string().into())],
            ),
        ),
    }
}

/// 安装执行完的结果：把主人填的值换成核心值并装上，卡片上的话按主人的语言写，给 Agent 的还是结构化的 JSON。
/// `writer` 是发卡片的 Bot（`None` 是主人自己在 App 里装的旧路径）；有它就把归属记下来。
#[allow(clippy::too_many_arguments)] // 卡片执行要用的几个值，再包一层也是逐个填。
pub(super) async fn install_ran(
    bus: Arc<Bus>,
    id: PluginId,
    package: PluginPackage,
    granted: Vec<Permission>,
    lan_webui: bool,
    secrets: SecretValues,
    settings: wonton_protocol::SettingValues,
    writer: Option<PluginWriter>,
    langs: Languages,
) -> Outcome {
    if let Err(e) = bus
        .plugins
        .store_input_values(&package.manifest, &secrets, None, &langs)
    {
        return Outcome::failed(
            e.kind(),
            l10n::text(
                &langs,
                "chat-install-failed",
                &[("kind", e.kind().into()), ("error", e.to_string().into())],
            ),
        );
    }
    let values = match card_settings(&package, &settings, &langs) {
        Ok(values) => values,
        Err(e) => {
            return Outcome::failed(e.kind(), e.to_string());
        }
    };
    match bus
        .plugins
        .install_detached(package, granted, lan_webui, values, langs.clone())
        .await
    {
        Ok(view) => {
            // 归属没记下来时更新就装不上，如实告诉 Agent。
            let note = match writer.map(|writer| bus.plugins.set_writer(&id, Some(writer))) {
                None | Some(Ok(())) => None,
                Some(Err(e)) => {
                    tracing::error!(plugin = %id, error = %e, "记插件的归属失败");
                    Some(l10n::text(
                        &langs,
                        "chat-install-note-writer",
                        &[("error", e.to_string().into())],
                    ))
                }
            };
            installed_outcome(&view, note, &langs)
        }
        Err(e) => Outcome::failed(
            e.kind(),
            l10n::text(
                &langs,
                "chat-install-failed-id",
                &[
                    ("id", id.as_str().into()),
                    ("kind", e.kind().into()),
                    ("error", e.to_string().into()),
                ],
            ),
        ),
    }
}

pub(super) fn installed_outcome(
    view: &PluginView,
    note: Option<String>,
    langs: &Languages,
) -> Outcome {
    let title = cards::text(&view.title);
    let mut result = json!({
        "installed": view.id,
        "title": title,
        "version": view.version,
        "status": view.status,
        "url": view.url,
    });
    let message = match &view.url {
        Some(url) => crate::l10n::text(
            langs,
            "chat-install-done-url",
            &[
                ("title", title.as_str().into()),
                ("url", url.as_str().into()),
            ],
        ),
        None => crate::l10n::text(
            langs,
            "chat-install-done",
            &[("title", title.as_str().into())],
        ),
    };
    let message = match note {
        Some(note) => {
            result["note"] = json!(note);
            crate::l10n::text(
                langs,
                "chat-outcome-with-note",
                &[("message", message.into()), ("note", note.into())],
            )
        }
        None => message,
    };
    Outcome::ok(result.to_string(), message)
}
