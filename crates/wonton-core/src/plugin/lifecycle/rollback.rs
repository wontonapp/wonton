//! 安装失败时的回滚：按创建的相反顺序撤掉本次安装创建的资源（见 [`super::Undo`]）。

use super::Undo;
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::PluginManager;

impl PluginManager {
    pub(in crate::plugin) async fn rollback(&self, undo: Undo, langs: &Languages) -> Vec<String> {
        let mut failures = Vec::new();
        self.rollback_containers(&undo, langs, &mut failures).await;
        self.rollback_bot_and_models(&undo, langs, &mut failures)
            .await;
        self.rollback_clients_and_volumes(&undo, langs, &mut failures)
            .await;
        self.rollback_networks(&undo, langs, &mut failures).await;
        self.rollback_files(&undo, langs, &mut failures).await;
        failures
    }

    async fn rollback_containers(
        &self,
        undo: &Undo,
        langs: &Languages,
        failures: &mut Vec<String>,
    ) {
        let mut note = note_for(langs, failures);
        for name in undo.containers.iter().rev() {
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-container",
                    &[("name", name.as_str().into())],
                ),
                self.engine
                    .remove_container(name)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
    }

    async fn rollback_bot_and_models(
        &self,
        undo: &Undo,
        langs: &Languages,
        failures: &mut Vec<String>,
    ) {
        let mut note = note_for(langs, failures);
        if let Some((id, bot)) = &undo.bot {
            // 收回主房间要用 Bot 的令牌，所以在吊销设备之前。
            if undo.handed_over {
                note(
                    l10n::text(
                        langs,
                        "plugin-install-rb-home-room",
                        &[("room", bot.account.home_room.as_str().into())],
                    ),
                    self.withdraw_home_room(bot).await,
                );
            }
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-bot-device",
                    &[("bot", bot.account.user_id.as_str().into())],
                ),
                self.revoke_bot_device(id, bot).await,
            );
        }
        if let Some(id) = &undo.models {
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-model-credentials",
                    &[("id", id.to_string().into())],
                ),
                self.revoke_models(id, langs)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        if let Some(id) = &undo.source {
            self.sources.remove(id);
        }
    }

    async fn rollback_clients_and_volumes(
        &self,
        undo: &Undo,
        langs: &Languages,
        failures: &mut Vec<String>,
    ) {
        let mut note = note_for(langs, failures);
        if let (Some(id), Some(hydra)) = (&undo.oidc_client, &self.oidc) {
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-oidc-client",
                    &[("id", id.as_str().into())],
                ),
                hydra.delete_client(id).await.map_err(|e| e.to_string()),
            );
        }
        for volume in &undo.volumes {
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-volume",
                    &[("volume", volume.as_str().into())],
                ),
                self.engine
                    .remove_volume(volume)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
    }

    async fn rollback_networks(&self, undo: &Undo, langs: &Languages, failures: &mut Vec<String>) {
        let mut note = note_for(langs, failures);
        if let Some(network) = &undo.lan_network {
            let result = self.engine.remove_network(network).await;
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-network",
                    &[("network", network.as_str().into())],
                ),
                result.map_err(|e| e.to_string()),
            );
        }
        if let Some(network) = &undo.network {
            if let (true, Some(core)) = (undo.core_attached, &self.cfg.core_container) {
                let result = self.engine.disconnect_network(network, core).await;
                note(
                    l10n::text(
                        langs,
                        "plugin-install-rb-core-leave-network",
                        &[("network", network.as_str().into())],
                    ),
                    result.map_err(|e| e.to_string()),
                );
            }
            let result = self.detach_relay(network).await;
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-egress-leave-network",
                    &[("network", network.as_str().into())],
                ),
                result.map_err(|e| e.to_string()),
            );
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-network",
                    &[("network", network.as_str().into())],
                ),
                self.engine
                    .remove_network(network)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
    }

    async fn rollback_files(&self, undo: &Undo, langs: &Languages, failures: &mut Vec<String>) {
        let mut note = note_for(langs, failures);
        if let Some(dir) = &undo.app_dir_created {
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-directory",
                    &[("dir", dir.display().to_string().into())],
                ),
                std::fs::remove_dir_all(dir).map_err(|e| e.to_string()),
            );
        }
        if let Some((id, before)) = &undo.secrets_before {
            let store = self.store.secrets();
            let restored = match before {
                Some(secrets) => store.save(id, secrets),
                None => store.remove(id),
            };
            note(
                l10n::text(
                    langs,
                    "plugin-install-rb-secret",
                    &[("id", id.to_string().into())],
                ),
                restored.map_err(|e| e.to_string()),
            );
        }
    }
}

/// 记一条回滚失败：`what` 是用 rb-* 消息拼好的资源名，`error` 是 Docker 的原话。
fn note_for<'a>(
    langs: &'a Languages,
    failures: &'a mut Vec<String>,
) -> impl FnMut(String, Result<(), String>) + 'a {
    move |what: String, result: Result<(), String>| {
        if let Err(e) = result {
            tracing::error!(%what, error = %e, "回滚失败");
            failures.push(l10n::text(
                langs,
                "plugin-install-rollback-item",
                &[("what", what.into()), ("error", e.into())],
            ));
        }
    }
}
