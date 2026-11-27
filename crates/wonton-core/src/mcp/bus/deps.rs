//! `Bus` 的依赖访问器：插件表、联系人、确认表、审计、语言，以及接手申请的冷却。

use super::*;

impl Bus {
    pub(super) fn matrix(&self) -> Option<&crate::matrix::Matrix> {
        self.matrix.get()?.matrix.as_deref()
    }

    pub(super) fn contacts(&self) -> Option<&crate::contacts::Contacts> {
        self.contacts.get().map(Arc::as_ref)
    }

    pub(super) fn cards(&self) -> Result<&Cards, String> {
        self.matrix
            .get()
            .ok_or_else(|| "service_unavailable: the core has no Matrix connection".to_owned())
    }

    pub(super) fn people(&self) -> Result<&People, String> {
        self.people
            .get()
            .ok_or_else(|| "service_unavailable: the core has no family records".to_owned())
    }

    pub(super) fn files(&self) -> Result<&Arc<Files>, Failure> {
        self.files
            .get()
            .ok_or_else(|| Failure::new(FileFailure::ServiceUnavailable, "核心没有启用文件服务"))
    }

    /// 插件的 MCP 客户端：文件服务读资源也用它，一个插件一条连接。
    pub fn upstreams(&self) -> Arc<Upstreams> {
        self.upstreams.clone()
    }

    /// 插件管理器：收件箱取发送方的 MCP 服务读页面用。
    pub fn plugins(&self) -> &Arc<PluginManager> {
        &self.plugins
    }

    pub fn inbox(&self) -> Option<Arc<crate::inbox::Inbox>> {
        self.inbox.get().cloned()
    }

    pub fn confirmations(&self) -> &Confirmations {
        &self.confirmations
    }

    pub(crate) fn agent_chains(&self) -> Option<&Arc<crate::gateway::agent_chains::AgentChains>> {
        self.agent_chains.get()
    }

    pub(crate) fn friends(&self) -> Option<&Arc<crate::friends::Friends>> {
        self.friends.get()
    }

    pub fn audit(&self) -> &Arc<Audit> {
        &self.audit
    }

    /// 接手申请在冷却中：返回还要等多久。过期的记录顺手清掉。
    pub(super) fn adopt_cooldown(&self, caller: &PluginId, id: &PluginId) -> Option<Duration> {
        let mut refusals = self.adopt_refusals.lock().expect("接手冷却表锁中毒");
        let key = (caller.clone(), id.clone());
        let now = Instant::now();
        let until = *refusals.get(&key)?;
        if until <= now {
            refusals.remove(&key);
            return None;
        }
        Some(until - now)
    }

    /// 记一次接手申请：冷却期内同一个 Bot 对同一个插件不再发卡，表里过期的顺手清掉。
    pub(super) fn note_adopt_attempt(&self, caller: &PluginId, id: &PluginId) {
        let mut refusals = self.adopt_refusals.lock().expect("接手冷却表锁中毒");
        refusals.retain(|_, until| *until > Instant::now());
        refusals.insert(
            (caller.clone(), id.clone()),
            Instant::now() + ADOPT_COOLDOWN,
        );
    }

    /// 主人最近一次请求用的语言：Agent 和 Bot 的调用没有请求头，文字按它写（ADR-0043）。
    pub fn owner_languages(&self) -> Languages {
        self.plugins.owner_languages()
    }

    /// 卡片文字用谁的语言：发给家人的按他最近一次请求的 `Accept-Language`（ADR-0043）；他还没打开过 App、
    /// 没有记录时按主人的语言（协商空列表会落到英文，不是主人的语言）。主人的卡片按主人的。
    pub fn languages_for(&self, recipient: Option<&str>) -> Languages {
        match recipient.and_then(|person| self.plugins.person_languages(person)) {
            Some(header) => Languages::from_header(Some(&header)),
            None => self.owner_languages(),
        }
    }
}
