//! `Bus` 的构造和 `attach_*`：总线把各依赖接上来的入口。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use crate::files::Files;
use crate::help::Help;
use crate::plugin::PluginManager;

use super::Takeovers;
use super::audit::Audit;
use super::{Bus, Cards, Confirmations, People, Upstreams};

impl Bus {
    pub fn new(
        plugins: Arc<PluginManager>,
        confirmations: Arc<Confirmations>,
        audit: Arc<Audit>,
    ) -> Arc<Self> {
        Self::with_upstreams(plugins, confirmations, audit, Arc::new(Upstreams::new()))
    }

    pub(super) fn with_upstreams(
        plugins: Arc<PluginManager>,
        confirmations: Arc<Confirmations>,
        audit: Arc<Audit>,
        upstreams: Arc<Upstreams>,
    ) -> Arc<Self> {
        let recover = confirmations.has_leftover_windows();
        confirmations.attach_activity(plugins.activity.clone());
        let bus = Arc::new(Self {
            plugins,
            confirmations,
            upstreams,
            audit,
            takeovers: Takeovers::new(),
            adopt_refusals: Mutex::new(HashMap::new()),
            pin_cards: Mutex::new(HashSet::new()),
            files: OnceLock::new(),
            help: OnceLock::new(),
            people: OnceLock::new(),
            matrix: OnceLock::new(),
            contacts: OnceLock::new(),
            agent_chains: OnceLock::new(),
            friends: OnceLock::new(),
            inbox: OnceLock::new(),
            peers: Mutex::new(Vec::new()),
        });
        bus.watch_plugins();
        if recover {
            bus.recover_windows();
        }
        bus
    }

    /// 接上文件服务。没接上时工具结果里的文件登记不了，Bot 的文件工具报 `service_unavailable`。
    pub fn attach_files(&self, files: Arc<Files>) {
        if self.files.set(files).is_err() {
            tracing::error!("文件服务接了两次，用先接上的");
        }
    }

    /// 接上帮助文档。没接上时 `wonton__help` 如实报错。
    pub fn attach_help(&self, help: Arc<Help>) {
        if self.help.set(help).is_err() {
            tracing::error!("帮助文档接了两次，用先接上的");
        }
    }

    /// 接上家人记录、角色记录和本服务器的 Matrix 服务器名（没接 Matrix 的核心是基础域名）。没接上时
    /// `wonton__user_locale` 如实报错，开卡片按没人有装卸权限算。
    pub fn attach_people(
        &self,
        server_name: String,
        members: Arc<crate::members::Members>,
        roles: Arc<crate::roles::Roles>,
    ) {
        if self
            .people
            .set(People {
                server_name,
                members,
                roles,
            })
            .is_err()
        {
            tracing::error!("家人记录接了两次，用先接上的");
        }
    }

    /// 接上 Matrix 和主人：代 Bot 查房间成员用。
    pub fn attach_matrix(&self, matrix: Option<Arc<crate::matrix::Matrix>>, owner: String) {
        let _ = self.matrix.set(Cards { matrix, owner });
    }

    /// 接上收件箱。没接时收件箱工具和接口如实报不可用。
    pub fn attach_inbox(&self, inbox: Arc<crate::inbox::Inbox>) {
        if self.inbox.set(inbox).is_err() {
            tracing::warn!("收件箱接了两次，用先接上的");
        }
    }

    /// 接上调用链的服务（ADR-0061 第 10 节）。只接一次。
    pub fn attach_agent_chains(&self, chains: Arc<crate::gateway::agent_chains::AgentChains>) {
        if self.agent_chains.set(chains).is_err() {
            tracing::warn!("调用链的服务接了两次，用先接上的");
        }
    }

    /// 接上好友名单（ADR-0051）。只接一次。
    pub fn attach_friends(&self, friends: Arc<crate::friends::Friends>) {
        if self.friends.set(friends).is_err() {
            tracing::warn!("好友名单接了两次，用先接上的");
        }
    }
}
