//! Bot 插件在 Matrix 上的副作用：在假 Docker 和假 Tuwunel 上验证安装、卸载、删除数据、回滚和核心重启。

use std::sync::Arc;

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{PluginId, PluginManager};
use super::manager::manager_in_container;
use super::package;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{Fake, start};
use wonton_protocol::BotKeeper;

#[path = "bot_web_lifecycle.rs"]
mod bot_web_lifecycle;
mod bots;
mod install;
mod listing;
mod restart;
mod settings;
mod uninstall;
mod updates;
mod upgrade;

/// 一个回声 Bot：唯一的服务拿到 Bot 的全部 Matrix 变量。
fn echo() -> String {
    package(
        "  bot:\n    image: echo\n    environment:\n      \
         HS: ${WONTON_MATRIX_HOMESERVER}\n      USER: ${WONTON_MATRIX_USER_ID}\n      \
         TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}\n      ALLOWED: ${WONTON_MATRIX_ALLOWED_USERS}\n      \
         ROOM: ${WONTON_MATRIX_HOME_ROOM}",
        "  bot: {display_name: {zh: 回声, en: Echo}}\n  permissions: [matrix.bot]",
    )
    .replace("id: app.test", "id: app.echo")
}

fn granted() -> Vec<super::super::Permission> {
    vec!["matrix.bot".parse().unwrap()]
}

/// 服务容器名：Bot 服务的容器带 `--<服务>--<Bot>` 后缀。
fn service_container(slug: &str, service: &str) -> String {
    super::super::names::Instance::default().bot_container(slug, service, "default")
}

struct Bed {
    docker: FakeDocker,
    engine: Engine,
    tuwunel: Fake,
    matrix: Arc<Matrix>,
    dir: tempfile::TempDir,
}

impl Bed {
    async fn start() -> Self {
        let (docker, engine) = FakeDocker::start().await;
        let (tuwunel, config) = start().await;
        let dir = tempfile::tempdir().unwrap();
        let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap();
        Self {
            docker,
            engine,
            tuwunel,
            matrix: Arc::new(matrix),
            dir,
        }
    }

    /// 核心（重新）启动后的插件管理器，数据目录不变。
    fn manager(&self) -> PluginManager {
        manager_in_container(self.engine.clone(), &self.dir).with_matrix(Some(self.matrix.clone()))
    }

    /// Bot 容器拿到的变量：名字 → 值。
    fn env(&self) -> std::collections::BTreeMap<String, String> {
        // 声明了 `bots` 的插件按 Bot 一份容器（带后缀），单 Bot 的插件还是普通服务名。
        let suffixed = service_container("app-echo", "bot");
        let env = if self.docker.created(&suffixed).is_some() {
            self.docker.env(&suffixed)
        } else {
            self.docker.env(&container_name("app-echo", "bot"))
        };
        env.iter()
            .filter_map(|e| e.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    fn token_valid(&self, token: &str) -> bool {
        self.tuwunel.state().tokens.contains_key(token)
    }
    fn source_address(&self) -> std::net::IpAddr {
        let body = self.docker.network("wonton-app-echo").unwrap();
        let subnet: ipnet::Ipv4Net = body["IPAM"]["Config"][0]["Subnet"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        std::net::Ipv4Addr::from(subnet.network().to_bits() + 9).into()
    }
}

fn id() -> PluginId {
    PluginId::parse("app.echo").unwrap()
}

/// 多 Bot 的 Agent：`bot` 服务每个 Bot 一份，挂自己的数据目录，拿到 Bot 名、总线地址和 Matrix 身份。
fn agent() -> String {
    package(
        "  bot:\n    image: echo\n    volumes: ['${WONTON_BOT_DATA}:/data']\n    environment:\n      \
         HS: ${WONTON_MATRIX_HOMESERVER}\n      USER: ${WONTON_MATRIX_USER_ID}\n      \
         TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}\n      ALLOWED: ${WONTON_MATRIX_ALLOWED_USERS}\n      \
         ROOM: ${WONTON_MATRIX_HOME_ROOM}\n      BOT: ${WONTON_BOT_NAME}\n      MCP: ${WONTON_MCP_URL}",
        "  bot: {display_name: {zh: 回声, en: Echo}}\n  agent: {protocol: matrix}\n  \
         bots: {services: [bot]}\n  permissions: [matrix.bot, tools]",
    )
    .replace("id: app.test", "id: app.echo")
    .replace("kind: app", "kind: agent")
}

fn granted_with_tools() -> Vec<super::super::Permission> {
    granted()
        .into_iter()
        .chain(["tools".parse().unwrap()])
        .collect()
}

fn bot_container(bot: &str) -> String {
    use super::super::names::Instance;
    Instance::default().bot_container("app-echo", "bot", bot)
}
