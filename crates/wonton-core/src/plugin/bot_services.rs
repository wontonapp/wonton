//! 一个插件多个 Bot（`x-wonton.bots`）：`bots.services` 里的服务每个 Bot 一份，有自己的 Bot 名和数据
//! 目录；其余服务全插件一份。M1 只有默认 Bot，按 Bot 实例化容器在 M2。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::compose::Service;
use super::interpolate::Vars;
use super::manifest::Manifest;

/// 安装时核心建的 Bot。
pub const DEFAULT_BOT: &str = "default";

/// 服务名之外，Bot 服务的另一个网络别名：同一个 Bot 的服务之间用它互访。
pub fn bot_alias(service: &str) -> String {
    format!("{service}-{DEFAULT_BOT}")
}

fn bot_services(manifest: &Manifest) -> BTreeSet<&str> {
    manifest
        .bots
        .iter()
        .flat_map(|b| &b.services)
        .map(String::as_str)
        .collect()
}

/// 每个服务插值用的变量表：Bot 服务多了 `WONTON_BOT_NAME` 和 `WONTON_BOT_DATA`
/// （`${WONTON_APP_DATA}/bots/<Bot 名>`），`WONTON_MODEL_API_KEY` 是这个 Bot 的凭据。声明了 `bots` 时 Bot 的
/// Matrix 身份（`WONTON_MATRIX_*`）也只给 Bot 服务：全插件一份的服务引用这些变量时安装失败。
pub struct ServiceVars<'a> {
    plugin: Vars,
    bot: Vars,
    bot_services: BTreeSet<&'a str>,
}

impl<'a> ServiceVars<'a> {
    /// `bot_model_key`：默认 Bot 的模型网关凭据，Bot 服务里的 `WONTON_MODEL_API_KEY` 用它。
    pub fn new(
        manifest: &'a Manifest,
        app_data: &Path,
        mut plugin: Vars,
        bot_model_key: Option<String>,
    ) -> Self {
        let mut bot = plugin.clone();
        bot.insert("WONTON_BOT_NAME".to_owned(), DEFAULT_BOT.to_owned());
        let data = app_data.join("bots").join(DEFAULT_BOT);
        bot.insert("WONTON_BOT_DATA".to_owned(), data.display().to_string());
        match bot_model_key {
            Some(key) => bot.insert("WONTON_MODEL_API_KEY".to_owned(), key),
            None => bot.remove("WONTON_MODEL_API_KEY"),
        };
        if manifest.bots.is_some() {
            plugin.retain(|name, _| !name.starts_with("WONTON_MATRIX_"));
        }
        Self {
            plugin,
            bot,
            bot_services: bot_services(manifest),
        }
    }

    pub fn get(&self, service: &str) -> &Vars {
        if self.bot_services.contains(service) {
            &self.bot
        } else {
            &self.plugin
        }
    }

    /// 服务在插件网络里的别名：服务名；Bot 服务另有 `<服务名>-<Bot 名>`，同一个 Bot 的服务之间用它互访。
    pub fn aliases(&self, service: &str) -> Vec<String> {
        let mut aliases = vec![service.to_owned()];
        if self.bot_services.contains(service) {
            aliases.push(bot_alias(service));
        }
        aliases
    }

    /// 全插件一份的服务不能依赖 Bot 服务：Bot 服务每个 Bot 一份，依赖哪一份说不清。返回（服务，被依赖的
    /// Bot 服务）。
    pub fn plugin_service_on_bot(
        &self,
        services: &BTreeMap<String, Service>,
    ) -> Option<(String, String)> {
        services
            .iter()
            .filter(|(name, _)| !self.bot_services.contains(name.as_str()))
            .find_map(|(name, service)| {
                let deps = service.depends_on.as_ref()?.entries();
                deps.into_iter()
                    .find(|(dep, _)| self.bot_services.contains(dep.as_str()))
                    .map(|(dep, _)| (name.clone(), dep))
            })
    }
}
