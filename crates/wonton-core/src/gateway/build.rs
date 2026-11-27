//! 组装 [`Gateway`]：构造与各子系统的接线。

use std::sync::atomic::Ordering;

use crate::config;

use super::*;

impl Gateway {
    /// `hydra` 带上身份提供方公开端点的地址，必须来自 [`crate::config::Config`]：配置层已经校验过它是
    /// `http://主机:端口`。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        public: PublicUrls,
        owner: Arc<OwnerStore>,
        owner_token: String,
        members: Arc<Members>,
        plugins: Arc<PluginManager>,
        hydra: Option<(Hydra, String)>,
        http: HttpClient,
        tunnel: Tunnel,
    ) -> Self {
        plugins.attach_person_members(members.clone());
        let (hydra, hydra_public) = match hydra {
            Some((hydra, public_url)) => {
                let authority = config::http_authority(&public_url)
                    .unwrap_or_else(|e| panic!("身份提供方的公开地址没有经过配置层校验：{e}"));
                (Some(hydra), Some(authority))
            }
            None => (None, None),
        };
        let onboarding = Arc::new(onboarding::OnboardingState::new(plugins.data_dir()));
        let link_previews = Arc::new(crate::link_preview::LinkPreviews::new(
            plugins.data_dir().join("core").join("link-previews"),
            plugins.core_proxy().clone(),
        ));
        let roles = Arc::new(Roles::initial());
        tunnel.pairing.attach_roles(roles.clone());
        plugins.attach_person_roles(roles.clone());
        Self {
            public,
            owner,
            owner_token,
            members,
            family_room_sync: tokio::sync::Mutex::new(()),
            member_sync: tokio::sync::Mutex::new(()),
            roles,
            clients: plugins.clients.clone(),
            plugins,
            hydra,
            hydra_public,
            http,
            system_http: HttpClient::system_loopback(),
            upstream_http: HttpClient::for_forwarding(),
            tunnel,
            matrix: None,
            send_limits: Arc::new(send_limits::SendLimiter::new(
                send_limits::SendLimits::default(),
            )),
            people_limits: Arc::new(people_limits::PeopleLimiter::new(
                people_limits::PeopleLimits::default(),
            )),
            push: None,
            mcp: None,
            notices: None,
            notify_limiter: notify_limit::Limiter::default(),
            files: None,
            media: None,
            audit: None,
            tusd: None,
            upgrades: None,
            peers: None,
            friends: None,
            friend_requests: friend_requests::Requests::default(),
            contacts: None,
            federation_blocks: None,
            sharing: None,
            onboarding,
            health_limits: health::RateLimits::new(),
            link_previews,
            app_calls: Arc::default(),
            domain: None,
            clients_apply: tokio::sync::Mutex::new(()),
            entry: None,
            public_access: Arc::new(AtomicBool::new(false)),
            public_ingress_configured: false,
            sessions: session::SessionStore::default(),
            agent_chains: None,
        }
    }

    #[must_use]
    pub fn with_clients(mut self, domain: Option<Arc<crate::clients::domain::Domain>>) -> Self {
        self.domain = domain;
        self
    }
    #[must_use]
    pub fn with_entry(mut self, entry: Option<Arc<crate::entry_access::EntryAdmin>>) -> Self {
        self.entry = entry;
        self
    }

    /// 和持久开关共用同一个标志：开关一改，读这个标志的路径立刻跟着变。
    #[must_use]
    pub fn with_public_access_flag(mut self, flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.public_access = flag;
        self
    }

    #[must_use]
    pub fn with_public_access(self, enabled: bool) -> Self {
        self.public_access.store(enabled, Ordering::Relaxed);
        self
    }

    /// 公网访问关着：入口监听对带 `X-Wonton-Entry: public` 的请求回 503。
    pub(crate) fn entry_closed(&self) -> bool {
        self.entry
            .as_ref()
            .is_some_and(|entry| !entry.access.enabled())
    }

    #[must_use]
    pub fn with_public_ingress_configured(mut self, configured: bool) -> Self {
        self.public_ingress_configured = configured;
        self
    }

    pub(crate) fn public_access_enabled(&self) -> bool {
        self.public_access.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn with_sharing(mut self, sharing: Option<Arc<sharing::Sharing>>) -> Self {
        self.sharing = sharing;
        self
    }

    /// 接上角色记录：网关自己只带模板时用 [`Roles::initial`]，生产由启动时装上 `roles.json`。
    #[must_use]
    pub fn with_roles(mut self, roles: Arc<Roles>) -> Self {
        self.tunnel.pairing.attach_roles(roles.clone());
        self.plugins.attach_person_roles(roles.clone());
        self.roles = roles;
        self
    }

    #[must_use]
    pub fn with_system_http(mut self, http: HttpClient) -> Self {
        self.system_http = http;
        self
    }

    /// 接上 Matrix 服务器：App 的登录令牌和 `matrix.<base>` 这才可用。
    #[must_use]
    pub fn with_matrix(mut self, matrix: Option<Arc<Matrix>>) -> Self {
        self.matrix = matrix;
        self
    }

    /// 换上配置里的 Bot 发消息限制（[`crate::config::Config`] 的环境变量改数字）；测试可以带自己的时钟。
    #[must_use]
    pub fn with_send_limiter(mut self, send_limits: Arc<send_limits::SendLimiter>) -> Self {
        self.send_limits = send_limits;
        self
    }

    /// 接上推送代理：设备登记推送、Tuwunel 调 `push.wonton.internal` 这才可用。
    #[must_use]
    pub fn with_push(mut self, push: Option<Arc<Push>>) -> Self {
        self.push = push;
        self
    }

    /// 接上 MCP 总线：`mcp.wonton.internal` 和回答确认的接口这才可用。
    #[must_use]
    pub fn with_mcp(mut self, mcp: Option<crate::mcp::Endpoint>) -> Self {
        self.mcp = mcp;
        self
    }

    /// 接上通知房间：`notify.wonton.internal` 这才可用（ADR-0047）。
    #[must_use]
    pub fn with_notices(mut self, notices: Option<Arc<crate::matrix::NoticeRoom>>) -> Self {
        self.notices = notices;
        self
    }

    /// 接上文件服务：`/_wonton/files/…` 这才可用。
    #[must_use]
    pub fn with_files(mut self, files: Option<Arc<crate::files::Files>>) -> Self {
        self.files = files;
        self
    }

    /// 接上媒体服务：`/_wonton/api/media/…` 这才可用。
    #[must_use]
    pub fn with_media(mut self, media: Option<Arc<crate::media::Media>>) -> Self {
        self.media = media;
        self
    }

    /// 接上审计日志：主人查看插件密钥的取值审计这才可用。
    #[must_use]
    pub fn with_audit(mut self, audit: Option<Arc<crate::mcp::audit::Audit>>) -> Self {
        self.audit = audit;
        self
    }

    /// 接上 tusd：`authority` 是 `host:port`，见 [`crate::config::http_authority`]。
    pub fn with_tusd(mut self, authority: Option<String>) -> Self {
        self.tusd = authority;
        self
    }

    /// 接上核心的一键升级：`/_wonton/api/core/upgrade` 这才可用。
    #[must_use]
    pub fn with_upgrades(
        mut self,
        upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
    ) -> Self {
        self.upgrades = upgrades;
        self
    }

    /// 接上服务器配对：`/_wonton/api/peers` 和联邦转发这才可用（ADR-0050）。没接 Matrix 的核心没有它。
    #[must_use]
    pub fn with_peers(mut self, peers: Option<Arc<crate::peers::Peers>>) -> Self {
        self.peers = peers;
        self
    }
}
