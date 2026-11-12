//! 已安装 Agent 的账号登录选项（ADR-0017 的“接口：App 和核心”），文字按请求语言挑选。

use super::languages::Languages;
use super::manager::PluginManager;
use super::manifest::{AgentSpec, PluginId};

impl PluginManager {
    /// 已安装 Agent 的账号登录选项，文字按请求语言挑选。
    pub fn agent_logins(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Vec<wonton_protocol::AgentLogin> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.agent.as_ref())
            .map(|agent| agent_login_options(agent, langs))
            .unwrap_or_default()
    }
}

pub(crate) fn agent_login_options(
    agent: &AgentSpec,
    langs: &Languages,
) -> Vec<wonton_protocol::AgentLogin> {
    agent
        .login
        .iter()
        .map(|login| wonton_protocol::AgentLogin {
            id: login.id.clone(),
            title: langs.pick_owned(&login.title),
        })
        .collect()
}
