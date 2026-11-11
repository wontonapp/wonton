//! 主人在 App 里查看插件密钥（#339）：清单里声明了 `show_owner` 的 `generate` 项。
//! HTTP 接口在 [`crate::gateway::owner_secrets`]，响应类型在 `wonton-protocol`。

use std::sync::Arc;

use wonton_protocol::{OwnerPluginSecret, OwnerPluginSecretValue, SecretString};

use super::languages::Languages;
use super::listed::Listed;
use super::manager::PluginManager;
use super::manifest::{PluginId, SecretSpec};

/// 主人可查看的密钥清单。
pub enum ViewableSecrets {
    /// 声明了 `show_owner` 的项，按名字排好（`secrets` 是 BTreeMap，遍历即名字序）。
    Listed(Vec<OwnerPluginSecret>),
    /// 插件装着，但记录读不出来，清单也给不出（详情页同样标注异常）。
    Unreadable,
}

/// 取一项密钥的值时按原因分的结果。
pub enum SecretValue {
    Value(OwnerPluginSecretValue),
    /// 插件装着，但记录读不出来，核对不了声明。
    Unreadable,
    /// 这个密钥没声明给主人看：没写 `show_owner`、`collect` 项，或清单里没有。
    NotViewable,
    /// 还没有值：密钥文件丢了且插件没启动，启动或升级一次会重新生成。
    NotGenerated,
    /// 核心读自己的密钥存储失败。
    Store(super::store::StoreError),
    /// 存的值不是文本（核心生成的都是文本，见到就是存储坏了）。
    NotText,
}

impl PluginManager {
    /// 清单里声明了 `show_owner` 的密钥：名字加按请求语言挑好的 label。没装时 `None`。
    pub fn owner_viewable_secrets(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Option<ViewableSecrets> {
        let listed = self.installed_listed(id)?;
        let Some(listed) = listed else {
            return Some(ViewableSecrets::Unreadable);
        };
        let secrets = listed
            .manifest
            .secrets
            .iter()
            .filter_map(|(name, spec)| {
                let SecretSpec::Generate(generate) = spec else {
                    return None;
                };
                let view = generate.view.as_ref()?;
                Some(OwnerPluginSecret {
                    name: name.clone(),
                    label: langs.pick_owned(&view.label),
                })
            })
            .collect();
        Some(ViewableSecrets::Listed(secrets))
    }

    /// 一项密钥的值，按原因分清。值不进审计，审计由接口层写。
    pub fn owner_secret_value(&self, id: &PluginId, name: &str) -> Option<SecretValue> {
        let listed = self.installed_listed(id)?;
        let Some(listed) = listed else {
            return Some(SecretValue::Unreadable);
        };
        let viewable = matches!(
            listed.manifest.secrets.get(name),
            Some(SecretSpec::Generate(generate)) if generate.view.is_some()
        );
        if !viewable {
            return Some(SecretValue::NotViewable);
        }
        let values = match self.store.secrets().load(id) {
            Ok(values) => values,
            Err(e) => return Some(SecretValue::Store(e)),
        };
        let bytes = values.and_then(|values| values.get(name).cloned());
        let Some(bytes) = bytes else {
            return Some(SecretValue::NotGenerated);
        };
        match String::from_utf8(bytes) {
            Ok(text) => Some(SecretValue::Value(OwnerPluginSecretValue {
                value: SecretString::new(text),
            })),
            Err(_) => Some(SecretValue::NotText),
        }
    }

    /// 装着的插件的清单：`None` 是没装，`Some(None)` 是装着但记录读不出来。
    fn installed_listed(&self, id: &PluginId) -> Option<Option<Arc<Listed>>> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        Some(plugins.get(id)?.listed.clone())
    }
}
