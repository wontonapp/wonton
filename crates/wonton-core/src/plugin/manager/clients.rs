//! 客户端入口的插件清单视图与持久开关。

use std::sync::Arc;

use wonton_protocol::{ClientAuthKind, PublicRuleMode, PublicRules};

use super::{InstalledRecord, LocalizedText, PluginId, PluginManager, PluginStatus};

#[derive(Debug, Clone)]
pub(crate) struct ClientPlugin {
    pub id: PluginId,
    pub title: LocalizedText,
    pub record: InstalledRecord,
    pub http: bool,
    pub auth: ClientAuthKind,
    pub username: Option<String>,
    pub public_rules: Option<PublicRules>,
    pub tcp_names: Vec<String>,
}
impl PluginManager {
    pub(crate) fn client_plugins(&self) -> Result<Vec<ClientPlugin>, String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut result = Vec::new();
        for (id, entry) in plugins.iter() {
            if matches!(entry.status, PluginStatus::Uninstalling) {
                continue;
            }
            let Some(listed) = &entry.listed else {
                continue;
            };
            let Some(clients) = &listed.manifest.clients else {
                continue;
            };
            if clients.http.is_none() && clients.tcp.as_ref().is_none_or(Vec::is_empty) {
                continue;
            }
            let record = self
                .store
                .load(id)
                .map_err(|error| format!("读取客户端入口记录 {id}: {error}"))?
                .ok_or_else(|| format!("客户端入口插件 {id} 没有安装记录"))?;
            let (http, auth, username, public_rules) = match clients.http.as_ref() {
                Some(http) => {
                    let auth = if http.auth.gateway.is_some() {
                        ClientAuthKind::Gateway
                    } else if http.auth.credentials.is_some() {
                        ClientAuthKind::Credentials
                    } else if http.auth.app.is_some() {
                        ClientAuthKind::App
                    } else {
                        ClientAuthKind::Other
                    };
                    let username = http.auth.credentials.as_ref().map(|credentials| {
                        credentials
                            .username
                            .clone()
                            .unwrap_or_else(|| "{user}".to_owned())
                    });
                    let public_rules = http.public.as_ref().map(|public| {
                        if let Some(paths) = &public.allow {
                            PublicRules {
                                mode: PublicRuleMode::Allow,
                                paths: paths.clone(),
                            }
                        } else {
                            PublicRules {
                                mode: PublicRuleMode::Deny,
                                paths: public.deny.clone().unwrap_or_default(),
                            }
                        }
                    });
                    (true, auth, username, public_rules)
                }
                None => (false, ClientAuthKind::Other, None, None),
            };
            let mut tcp_names: Vec<String> = clients
                .tcp
                .as_ref()
                .into_iter()
                .flatten()
                .map(|entry| entry.name.clone())
                .collect();
            tcp_names.sort();
            result.push(ClientPlugin {
                id: id.clone(),
                title: listed.manifest.title.clone(),
                record,
                http,
                auth,
                username,
                public_rules,
                tcp_names,
            });
        }
        Ok(result)
    }

    pub fn attach_clients_gateway(&self, gateway: &Arc<crate::gateway::Gateway>) {
        let _ = self.clients_gateway.set(Arc::downgrade(gateway));
    }

    pub(in crate::plugin) fn clients_gateway(&self) -> Option<Arc<crate::gateway::Gateway>> {
        self.clients_gateway
            .get()
            .and_then(std::sync::Weak::upgrade)
    }

    /// 和其他插件记录读改写共用，调用方持锁时不得再调用 setup 检查。
    pub(crate) async fn clients_record_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lan.pass.lock().await
    }

    pub(crate) fn publish_client_entries(&self) {
        self.changed();
    }

    pub(crate) fn update_client_entry(
        &self,
        id: &PluginId,
        kind: crate::clients::state::Kind,
        enabled: bool,
    ) -> Result<InstalledRecord, String> {
        let mut record = self
            .store
            .load(id)
            .map_err(|error| format!("读取客户端入口记录 {id}: {error}"))?
            .ok_or_else(|| format!("客户端入口插件 {id} 没有安装记录"))?;
        let old = record.clone();
        match kind {
            crate::clients::state::Kind::Lan => record.clients_entries.lan = enabled,
            crate::clients::state::Kind::Public => record.clients_entries.public = enabled,
        }
        self.store
            .save(&record)
            .map_err(|error| format!("保存客户端入口记录 {id}: {error}"))?;
        if !enabled {
            self.changed();
        }
        Ok(old)
    }

    pub(crate) fn restore_client_entry(&self, record: &InstalledRecord) -> Result<(), String> {
        let mut current = self
            .store
            .load(&record.id)
            .map_err(|error| format!("读取回滚记录 {}: {error}", record.id))?
            .ok_or_else(|| format!("回滚客户端入口记录 {} 时插件记录已不存在", record.id))?;
        current.clients_entries = record.clients_entries;
        self.store
            .save(&current)
            .map_err(|error| format!("回滚客户端入口记录 {}: {error}", record.id))?;
        self.changed();
        Ok(())
    }
}
