//! 一个插件多个 Bot（`x-wonton.bots`）：`bots.services` 里的服务每个 Bot 一份，有自己的 Bot 名、数据目录、
//! Matrix 身份、模型凭据和总线令牌；其余服务全插件一份。安装时只建默认 Bot，其余的经 [`super::manager`]
//! 的 `add_bot` 建。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::compose::Service;
use super::interpolate::Vars;
use super::manifest::Manifest;
use super::render::MatrixVars;
use wonton_protocol::{AiLevel, BOT_AI_LEVEL_ENV};

/// 安装时核心建的 Bot。
pub const DEFAULT_BOT: &str = "default";

/// 一个 Bot 渲染容器时要注入的、和其他 Bot 不一样的值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotVars {
    /// 这个 Bot 的模型网关凭据（`<插件 id>/<Bot 名>`），Bot 服务里的 `WONTON_MODEL_API_KEY` 用它。
    pub model_key: Option<String>,
    /// 这个 Bot 的 Matrix 身份（`WONTON_MATRIX_*`）。
    pub matrix: Option<MatrixVars>,
    /// 这个 Bot 的总线地址，令牌在路径里：`http://mcp.wonton.internal:<端口>/mcp/<令牌>`。
    pub mcp_url: Option<String>,
    pub app_url: Option<String>,
    pub oidc: Option<super::vars::OidcVars>,
    /// 这个 Bot 自己的能力连接变量；不继承插件级确认和凭据。
    pub dependencies: BTreeMap<String, String>,
    /// 这个 Bot 的 AI 动手级别（`WONTON_BOT_AI_LEVEL`，ADR-0088 第 8 节）：私人 Bot 是归属人角色的，
    /// 共享 Bot 是能用人里最低的。Agent 插件和桥凭它锁模式。
    pub ai_level: AiLevel,
    /// 归属人“AI 动手”是只读的私人 Bot：共享文件夹挂载一律只读（ADR-0088 第 5 节第 1 层），
    /// 插件自己声明了写权限的也一样。共享 Bot 没有归属人，总是 false。
    pub keeper_read_only: bool,
}

impl Default for BotVars {
    fn default() -> Self {
        Self {
            model_key: None,
            matrix: None,
            mcp_url: None,
            app_url: None,
            oidc: None,
            dependencies: BTreeMap::new(),
            // 解析不出的级别按只读（ADR-0088 第 5 节），和 [`crate::ai_level`] 的兜底一致。
            ai_level: AiLevel::ReadOnly,
            keeper_read_only: false,
        }
    }
}

/// 服务名之外，Bot 服务的另一个网络别名：同一个 Bot 的服务之间用它互访。
pub fn bot_alias(service: &str, bot: &str) -> String {
    format!("{service}-{bot}")
}

/// 清单声明的 Bot 服务名（`bots.services`）。
pub(super) fn bot_services(manifest: &Manifest) -> BTreeSet<&str> {
    manifest
        .bots
        .iter()
        .flat_map(|b| &b.services)
        .map(String::as_str)
        .collect()
}

/// 每个服务插值用的变量表。声明了 `bots` 时，全插件一份的服务拿不到任何一个 Bot 的 Matrix 身份和总线地址
/// （每个 Bot 一份），引用了就安装失败；Bot 服务按 Bot 各有一套。
pub struct ServiceVars<'a> {
    plugin: Vars,
    bots: BTreeMap<String, Vars>,
    bot_services: BTreeSet<&'a str>,
    slug: String,
}

impl<'a> ServiceVars<'a> {
    /// `bots`：每个 Bot 的 [`BotVars`]，按 Bot 名。没有 `bots` 的插件传空表，全插件一份的服务用插件级的
    /// 变量（默认 Bot 的 Matrix 身份和总线地址就在里面）。
    pub fn new(
        manifest: &'a Manifest,
        app_data: &Path,
        mut plugin: Vars,
        bots: &BTreeMap<String, BotVars>,
    ) -> Self {
        let mut per_bot = BTreeMap::new();
        if manifest.bots.is_some() {
            plugin
                .retain(|name, _| !name.starts_with("WONTON_MATRIX_") && name != "WONTON_MCP_URL");
        }
        let dependency_names: Vec<_> = super::dependencies::declarations(manifest)
            .flat_map(|capability| {
                let stem = super::capabilities::variable_stem(capability);
                ["URL", "KEY", "HOST", "PORT"].map(|suffix| format!("WONTON_{stem}_{suffix}"))
            })
            .collect();
        for (bot, runtime) in bots {
            let mut vars = plugin.clone();
            for name in &dependency_names {
                vars.remove(name);
            }
            vars.extend(
                runtime
                    .dependencies
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            if let Some(url) = &runtime.app_url {
                vars.insert("WONTON_APP_URL".to_owned(), url.clone());
            }
            if let Some(oidc) = &runtime.oidc {
                vars.insert("WONTON_OIDC_CLIENT_ID".to_owned(), oidc.client_id.clone());
                vars.insert(
                    "WONTON_OIDC_CLIENT_SECRET".to_owned(),
                    oidc.client_secret.clone(),
                );
            }
            vars.insert("WONTON_BOT_NAME".to_owned(), bot.clone());
            vars.insert(
                BOT_AI_LEVEL_ENV.to_owned(),
                runtime.ai_level.as_str().to_owned(),
            );
            let data = app_data.join("bots").join(bot);
            vars.insert("WONTON_BOT_DATA".to_owned(), data.display().to_string());
            match &runtime.model_key {
                Some(key) => vars.insert("WONTON_MODEL_API_KEY".to_owned(), key.clone()),
                None => vars.remove("WONTON_MODEL_API_KEY"),
            };
            match &runtime.matrix {
                Some(matrix) => {
                    vars.insert(
                        "WONTON_MATRIX_HOMESERVER".to_owned(),
                        matrix.homeserver.clone(),
                    );
                    vars.insert("WONTON_MATRIX_USER_ID".to_owned(), matrix.user_id.clone());
                    vars.insert(
                        "WONTON_MATRIX_ACCESS_TOKEN".to_owned(),
                        matrix.access_token.clone(),
                    );
                    vars.insert(
                        "WONTON_MATRIX_ALLOWED_USERS".to_owned(),
                        matrix.allowed_users.clone(),
                    );
                    vars.insert(
                        "WONTON_MATRIX_HOME_ROOM".to_owned(),
                        matrix.home_room.clone(),
                    );
                }
                None => {
                    for name in [
                        "WONTON_MATRIX_HOMESERVER",
                        "WONTON_MATRIX_USER_ID",
                        "WONTON_MATRIX_ACCESS_TOKEN",
                        "WONTON_MATRIX_ALLOWED_USERS",
                        "WONTON_MATRIX_HOME_ROOM",
                    ] {
                        vars.remove(name);
                    }
                }
            }
            match &runtime.mcp_url {
                Some(url) => vars.insert("WONTON_MCP_URL".to_owned(), url.clone()),
                None => vars.remove("WONTON_MCP_URL"),
            };
            per_bot.insert(bot.clone(), vars);
        }
        Self {
            plugin,
            bots: per_bot,
            bot_services: bot_services(manifest),
            slug: manifest.id.slug(),
        }
    }

    /// 全插件一份的服务的变量。
    pub fn get(&self, service: &str) -> &Vars {
        if self.bot_services.contains(service) {
            self.bots.values().next().unwrap_or(&self.plugin)
        } else {
            &self.plugin
        }
    }

    /// 一个 Bot 的服务的变量；`bot` 不存在时为 `None`。
    pub fn get_bot(&self, bot: &str) -> Option<&Vars> {
        self.bots.get(bot)
    }

    pub fn is_bot_service(&self, service: &str) -> bool {
        self.bot_services.contains(service)
    }

    /// Bot 服务的服务名。
    pub fn bot_service_names(&self) -> &BTreeSet<&'a str> {
        &self.bot_services
    }

    /// 要建容器的 Bot，按名字排序。
    pub fn bot_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.bots.keys().map(String::as_str)
    }

    /// 服务在插件网络里的别名：服务名；Bot 服务另有 `<服务名>-<Bot 名>`，同一个 Bot 的服务之间用它互访；
    /// 再加 `<服务名>.<slug>.wonton.internal`（[`super::names::service_host`]），核心连上游的 Host，
    /// 插件经 `WONTON_SERVICE_DOMAIN` 把它写进上游服务的 Host 白名单。
    pub fn aliases(&self, service: &str, bot: Option<&str>) -> Vec<String> {
        let mut aliases = vec![service.to_owned()];
        if let Some(bot) = bot
            && self.bot_services.contains(service)
        {
            aliases.push(bot_alias(service, bot));
            aliases.push(super::names::service_host(
                &self.slug,
                &bot_alias(service, bot),
            ));
        }
        aliases.push(super::names::service_host(&self.slug, service));
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
