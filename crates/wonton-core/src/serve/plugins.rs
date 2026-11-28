//! 插件的启动接线：引擎、预装、管理器、MCP 总线、调用链、帮助文档。
use super::*;

pub(crate) fn check_host_dev(cfg: &Config) -> anyhow::Result<()> {
    if let Some(dev) = &cfg.host_dev
        && !dev.is_dir()
    {
        bail!(
            "WONTON_HOST_DEV 指向的 {} 不是目录：核心在容器里时要把宿主的 /dev 只读挂载到这里",
            dev.display()
        );
    }
    // udev 数据库不是每个宿主都有（没装 udev 的最小系统），缺了照常启动。
    if let Some(udev) = &cfg.host_udev
        && !udev.is_dir()
    {
        tracing::warn!(path = %udev.display(), "WONTON_HOST_UDEV 指向的不是目录，设备插件的自动发现可能用不了");
    }
    Ok(())
}

pub(crate) fn plugin_engine(cfg: &Config) -> anyhow::Result<Engine> {
    let detector = match &cfg.core_container {
        Some(core) => lan::Detector::Container { core: core.clone() },
        None => lan::Detector::Here,
    };
    Ok(Engine::connect()?.with_network_pool(cfg.plugin_network_pool, detector))
}

/// 在后台预装（要拉镜像、等健康，不挡着网关启动），顺序见 [`PluginManager::preinstall_bundled`]。
pub(crate) fn preinstall(
    cfg: &Config,
    manager: Arc<PluginManager>,
    upgrades: Arc<Upgrades>,
) -> anyhow::Result<()> {
    let (system, defaults) = (cfg.system_plugins.clone(), cfg.default_plugins.clone());
    for (name, dir) in [
        ("WONTON_SYSTEM_PLUGINS", &system),
        ("WONTON_DEFAULT_PLUGINS", &defaults),
    ] {
        if let Some(dir) = dir
            && !dir.is_dir()
        {
            bail!("{name} 指向的 {} 不是目录", dir.display());
        }
    }
    let backoff = Backoff::default();
    tokio::spawn(async move {
        upgrades.wait_released().await;
        manager
            .preinstall_bundled(system.as_deref(), defaults.as_deref(), backoff)
            .await;
    });
    Ok(())
}

/// 主人的语言跟着管理器走：App 里改了语言，卡片和通知跟着换。
pub(crate) fn owner_languages(
    manager: &Arc<PluginManager>,
) -> Arc<dyn Fn() -> Languages + Send + Sync> {
    let manager = manager.clone();
    Arc::new(move || manager.owner_languages())
}

/// 和主人的通知房间（ADR-0016）：插件通知发进这里，确认卡片不进（ADR-0016 第 6 条）。没接 Matrix 时插件通知如实报不可用。
pub(crate) fn notice_room(
    cfg: &Config,
    matrix: Option<&Arc<Matrix>>,
    languages: Arc<dyn Fn() -> Languages + Send + Sync>,
) -> Option<Arc<NoticeRoom>> {
    matrix.map(|matrix| {
        let saved = cfg.core_state_dir().join("matrix-notice-room");
        Arc::new(NoticeRoom::new(
            matrix.clone(),
            &cfg.username,
            saved,
            languages,
        ))
    })
}

/// 装配插件管理器：引擎、来源、局域网与出网设置、镜像加速、升级冻结。
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_plugin_manager(
    cfg: &Config,
    engine: Engine,
    store: Store,
    http: HttpClient,
    hydra: Option<Hydra>,
    matrix: Option<Arc<Matrix>>,
    owner: Arc<OwnerStore>,
    upgrades: &Upgrades,
    images: Arc<ImageWorker>,
) -> PluginManager {
    PluginManager::new(
        engine,
        store,
        http,
        hydra,
        lifecycle_config(cfg, owner),
        images,
    )
    .with_matrix(matrix)
    .with_host(HostPaths::new(cfg.host_dev.clone(), cfg.host_udev.clone()))
    .with_lan(Lan::from_config(cfg))
    .with_egress(
        Egress::new(cfg.egress_proxy.clone(), cfg.egress_proxy_detected.clone())
            .with_doh(cfg.doh_url.clone()),
    )
    .with_store(cfg.store.clone())
    .with_upgrade_freeze(upgrades.frozen())
}

/// 把总线认人用的资料和 Matrix 接上：家人记录、服务器名，以及代 Bot 查房间成员用的 Matrix。
pub(crate) fn attach_identity(
    bus: &Arc<mcp::Bus>,
    matrix: Option<&Arc<Matrix>>,
    cfg: &Config,
    server_name: String,
    members: Arc<Members>,
    roles: Arc<Roles>,
    push: Arc<wonton_core::push::Push>,
) {
    bus.attach_people(server_name, members.clone(), roles.clone());
    bus.attach_matrix(matrix.cloned(), cfg.username.clone());
    let admins = Arc::new(MemberAdmins {
        owner: cfg.username.clone(),
        members,
        roles,
    });
    let dir = cfg.core_state_dir().join("inbox");
    match wonton_core::inbox::Inbox::open(dir, Arc::new(wonton_core::inbox::SystemClock), admins) {
        Ok(inbox) => {
            inbox.attach_devices(Arc::new(PushDevices(push)));
            bus.attach_inbox(inbox);
        }
        Err(error) => tracing::warn!(%error, "收件箱打不开，收件箱工具和接口不可用"),
    }
}

struct PushDevices(Arc<wonton_core::push::Push>);

impl wonton_core::inbox::DevicePresence for PushDevices {
    fn has_devices(&self, person: Option<&str>) -> bool {
        self.0.has_devices(person)
    }
}

/// 管理员名单：主人，或有“管理成员”权限的已加入成员。
struct MemberAdmins {
    owner: String,
    members: Arc<Members>,
    roles: Arc<Roles>,
}

impl wonton_core::inbox::AdminRoster for MemberAdmins {
    fn is_admin(&self, username: &str) -> bool {
        if username == self.owner {
            return true;
        }
        let Some(member) = self.members.get(username) else {
            return false;
        };
        if member.status != wonton_protocol::MemberStatus::Joined {
            return false;
        }
        self.roles
            .get(&member.role_id)
            .is_some_and(|role| role.permissions.manage_members)
    }
}

/// MCP 总线（ADR-0016）：开卡时给收件人的设备发推送，之后状态变化只改确认表，设备从
/// “等你确认”列表的长轮询里知道。卡片不进任何 Matrix 房间。
pub(crate) fn mcp_bus(
    cfg: &Config,
    manager: Arc<PluginManager>,
    audit: Arc<Audit>,
    push: Arc<Push>,
) -> Arc<mcp::Bus> {
    let notifier: Arc<dyn mcp::Notifier> = Arc::new(mcp::DevicePush::new(push));
    let confirmations = Arc::new(
        mcp::Confirmations::new(notifier, cfg.confirmation_ttl).with_windows(
            cfg.confirmation_window,
            cfg.core_state_dir().join("confirmation-windows.json"),
        ),
    );
    mcp::Bus::new(manager, confirmations, audit)
}

/// 调用链的服务（ADR-0061 第 10 节）：读回设置和账本，把“用到上限”接到上限卡片上。
/// 账本读不出来就启动失败，不拿空账本顶替。
pub(crate) fn open_agent_chains(
    cfg: &Config,
    bus: &Arc<mcp::Bus>,
) -> anyhow::Result<Arc<AgentChains>> {
    let settings_path = cfg.core_state_dir().join("agent-settings.json");
    let settings = Arc::new(AgentSettings::open(&settings_path)?);
    let ledger = Arc::new(Chains::open(
        cfg.core_state_dir().join("agent-chains.json"),
        settings.limits(),
    )?);
    ledger.on_limit_event(gateway::agent_chains::limit_handler(bus));
    let outbox = Arc::new(Outbox::with_store(
        cfg.core_state_dir().join("agent-outbox.json"),
    )?);
    outbox.on_message_settled(message_settled_audit(bus));
    Ok(Arc::new(AgentChains::new(ledger, outbox, settings)))
}

/// 出箱的 Message 项结清时写一行审计：发出的事件 ID、或没做成的类别（ADR-0061 第 9 节）。
pub(crate) fn message_settled_audit(
    bus: &Arc<mcp::Bus>,
) -> Arc<dyn Fn(&wonton_core::agent_outbox::MessageSettled) + Send + Sync> {
    let audit = bus.audit().clone();
    Arc::new(move |settled: &wonton_core::agent_outbox::MessageSettled| {
        let outcome = match &settled.result {
            wonton_protocol::OutboxResult::Sent { event_id } => format!("已发出，事件 {event_id}"),
            wonton_protocol::OutboxResult::Failed { category } => {
                format!("没做成（{category:?}）")
            }
        };
        let arguments = serde_json::json!({
            "recipient": settled.recipient,
            "body": settled.body,
        });
        audit.record(&mcp::audit::Record {
            caller: settled.bot.plugin_id.as_str(),
            tool: wonton_protocol::SEND_MESSAGE_TOOL,
            arguments: &arguments,
            decision: mcp::audit::Decision::Allowed,
            basis: Some(mcp::audit::Grounds {
                basis: mcp::audit::Basis::Card,
                held_ms: None,
            }),
            outcome: Some(&outcome),
            files: &[],
        });
    })
}

/// 用安装脚本装的核心（有安装目录）才管入口和自己的域名。
pub(crate) fn help_docs(
    cfg: &Config,
    manager: Arc<PluginManager>,
    upgrades: Arc<Upgrades>,
) -> Arc<wonton_core::help::Help> {
    let version: wonton_core::help::CoreVersion = Arc::new(move || {
        let upgrades = upgrades.clone();
        Box::pin(async move { upgrades.running_version().await })
    });
    wonton_core::help::Help::new(cfg.docs.clone(), manager, version)
}

/// 装配插件管理器：恢复已安装的插件、起监控、装默认插件（ADR-0025 的预装在后台跑）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_plugin_manager(
    cfg: &Config,
    engine: Engine,
    upgrades: Arc<Upgrades>,
    http: &HttpClient,
    hydra: Option<Hydra>,
    matrix: &Option<Arc<Matrix>>,
    owner: Arc<OwnerStore>,
    members: Arc<Members>,
    roles: Arc<Roles>,
    sharing: &sharing_start::Registries,
    mailpit: Option<Arc<wonton_core::mailbox::Component>>,
) -> anyhow::Result<(Arc<PluginManager>, Arc<ImageWorker>)> {
    let store = Store::open(cfg.core_state_dir().join("plugins"))?;
    let images = ImageWorker::current_in(store.people_languages(), cfg.username.clone())
        .context("找不到核心自己的可执行文件（处理图片要用）")?;
    // 助手要在隧道绑 UDP 之前起好：之后图片任务的子进程都在它里面 fork，核心不再在 iroh 重绑的窗口里
    // fork（#66）。起不来不挡住启动，图片任务会如实报不可用，预装会重试。
    if let Err(e) = images.start().await {
        tracing::warn!("图片助手没起来，图片处理要等它重起：{e}");
    }
    let images = Arc::new(images);
    let matrix_rtc_ca = matrix
        .as_ref()
        .map(|_| wonton_core::plugin::matrix_rtc_ca::CoreCa::load_or_create(&cfg.core_state_dir()))
        .transpose()?;
    let manager = Arc::new(
        open_plugin_manager(
            cfg,
            engine,
            store,
            http.clone(),
            hydra,
            matrix.clone(),
            owner.clone(),
            &upgrades,
            images.clone(),
        )
        .with_mailpit(mailpit)
        .with_matrix_rtc_ca(matrix_rtc_ca),
    );
    if let Some(dir) = &cfg.default_plugins {
        manager.set_default_bot_plugin(default_bot_plugin_in(dir).context("读取默认插件目录")?);
    }
    manager.attach_person_state(members, roles);
    manager.attach_sharing(sharing.friends.clone(), sharing.shares.clone());
    manager.spawn_registry_mirror_probe();
    // 启动时把宿主主机名取好缓存起来，装插件、重建容器时不再查 Docker；没取到时下次安装再取。
    manager.host_name().await;
    Ok((manager, images))
}
/// 网关开始接连接之后才做：核对已装插件、起监控、装默认插件。
pub(crate) async fn start_restored_plugins(
    cfg: &Config,
    manager: &Arc<PluginManager>,
    notices: Option<Arc<NoticeRoom>>,
    upgrades: Arc<Upgrades>,
) -> anyhow::Result<()> {
    manager.restore().await.context("恢复已安装的插件")?;
    spawn_plugin_monitors(manager, notices);
    upgrades.spawn();
    preinstall(cfg, manager.clone(), upgrades)?;
    Ok(())
}

pub(crate) fn spawn_federation_config(
    matrix: Option<&Arc<Matrix>>,
    cfg: &Config,
    peers: &Option<Arc<peers::Peers>>,
    notices: &Option<Arc<NoticeRoom>>,
    blocks: Option<Arc<FederationBlocks>>,
) {
    let Some(names) = peers.as_ref().map(|peers| peers.paired_names()) else {
        return;
    };
    let (Some(server_name), Some(blocks)) =
        (cfg.matrix.as_ref().map(|m| m.server_name.clone()), blocks)
    else {
        return;
    };
    FederationConfig::spawn(
        matrix.cloned(),
        cfg.install_dir.clone(),
        cfg.data_dir.clone(),
        notices.clone(),
        names,
        server_name,
        blocks,
    );
}
