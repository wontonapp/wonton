//! 测试用：登记一个声明了 `agent.models` 的 Agent 插件，命令按排好的输出回。插件和网关的测试共用。

use std::sync::{Arc, Mutex};

use serde_json::Value;
use wonton_protocol::BotKeeper;

use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::manifest::Manifest;
use super::super::{PluginManager, PluginPackage, PluginStatus};

pub(crate) const PLUGIN: &str = "app.agent";
/// 声明模型命令的那一段，拼进 `agent: {protocol: matrix…}`。
pub(crate) const MODELS: &str = ", models: {service: agent, command: [wonton-models, --json]}";

pub(crate) fn manifest(agent_extra: &str) -> Manifest {
    let yaml = format!(
        "services:\n  agent:\n    image: agent\nx-wonton:\n  spec: '0.17'\n  id: {PLUGIN}\n  \
         version: 1.0.0\n  kind: agent\n  title: {{zh: 助手, en: Agent}}\n  description: {{zh: 助手, \
         en: Agent}}\n  agent: {{protocol: matrix{agent_extra}}}\n  permissions: [matrix.bot]\n  \
         bots: {{services: [agent]}}\n"
    );
    PluginPackage::manifest_of(&yaml, &Languages::default()).unwrap()
}

/// 一次命令：在哪个容器、跑什么、标准输入那一行。
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub container: String,
    pub argv: Vec<String>,
    pub input: Value,
}

pub(crate) struct Agent {
    seen: Arc<Mutex<Vec<Seen>>>,
    reply: Arc<Mutex<(i64, String, String)>>,
}

impl Agent {
    /// 之后每次命令都这样回：退出码、标准输出、标准错误。
    pub fn reply(&self, code: i64, stdout: &str, stderr: &str) {
        *self.reply.lock().unwrap() = (code, stdout.to_owned(), stderr.to_owned());
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

pub(crate) fn container(manager: &PluginManager, bot: &str) -> String {
    manager
        .cfg
        .instance
        .bot_container("app-agent", "agent", bot)
}

/// 登记插件（运行中）和它的 Bot，给每个 Bot 放好容器；命令默认退出 0、输出空。
pub(crate) fn install(
    manager: &PluginManager,
    docker: &FakeDocker,
    agent_extra: &str,
    bots: &[(&str, BotKeeper)],
) -> Agent {
    let manifest = manifest(agent_extra);
    let id = manifest.id.clone();
    manager.register_for_test(manifest, PluginStatus::Running, None);
    let users: Vec<String> = bots.iter().map(|(bot, _)| format!("@{bot}:test")).collect();
    let entries: Vec<(&str, &str, BotKeeper, &str)> = bots
        .iter()
        .zip(&users)
        .map(|((bot, keeper), user)| (*bot, user.as_str(), keeper.clone(), "bus"))
        .collect();
    manager.bot_records_for_test(&id, &entries);
    for (bot, _) in bots {
        docker.add_container(&container(manager, bot), PLUGIN);
    }
    let agent = Agent {
        seen: Arc::default(),
        reply: Arc::new(Mutex::new((0, String::new(), String::new()))),
    };
    let (seen, reply) = (agent.seen.clone(), agent.reply.clone());
    docker.on_exec_output(move |container, argv, stdin| {
        seen.lock().unwrap().push(Seen {
            container: container.to_owned(),
            argv: argv.to_vec(),
            input: serde_json::from_slice(stdin).unwrap_or(Value::Null),
        });
        reply.lock().unwrap().clone()
    });
    agent
}
