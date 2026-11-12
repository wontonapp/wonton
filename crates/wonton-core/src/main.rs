//! `wonton-core`：服务端核心进程。
//!
//! - `wonton-core serve`（默认）：启动插件管理器、网关和隧道入口。
//! - `wonton-core healthcheck`：请求本机网关的健康端点，供容器健康检查使用。
//! - `wonton-core init <安装目录> <基础域名>`：正式安装时生成密钥、Matrix 服务器名、Tuwunel 配置和 compose 文件
//!   （`wonton_core::install`）。
//! - `wonton-core pair [--first] [IP:端口…]`：向运行中的核心要配对码，在终端里显示二维码（见 `pair_command`）。
//! - `wonton-core plugin …`：插件检查、打包、索引与构建期锁文件取包（见 `plugin_command`）。
//! - `wonton-core image <icon.png|icon.svg|screenshot>`：核心内部用，在子进程里处理插件的图片
//!   （`wonton_core::plugin::image_worker`）。
//! - `wonton-core image-host <程序>`：核心内部用，常驻的图片助手，由它起上面那些处理图片的子进程；
//!   核心启动时、绑隧道之前就先起它（`wonton_core::plugin::image_worker`）。
//! - `wonton-core lan-probe`：核心内部用，在宿主网络里的一次性容器里读宿主的路由和网卡
//!   （`wonton_core::plugin::lan::probe`）；`lan-probe listen <地址:端口,…>` 报告哪些端口有程序在用。
//! - `wonton-core egress-proxy-check`：查代理连不连得上，安装脚本和核心都在出口网络上的一次性容器里跑它
//!   （`wonton_core::plugin::egress::check`）。
//! - `wonton-core updater <安装目录> <数据目录>`：核心内部用，升级器容器（`wonton_core::upgrade::updater`）。
//! - `wonton-core compose-images`：升级器在新镜像里执行，列出这个版本要用的其他镜像。
//! - `wonton-core manual-rollback …`：`install.sh` 手动退回时在目标镜像里执行（`wonton_core::upgrade::manual`）。

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use tracing_subscriber::EnvFilter;
use wonton_core::agent_chain::Chains;
use wonton_core::agent_chain::settings::AgentSettings;
use wonton_core::agent_outbox::Outbox;
use wonton_core::config::Config;
mod serve;
use wonton_core::device::Devices;
use wonton_core::files::{CoreHost, Files, FilesConfig};
use wonton_core::friends::Friends;
use wonton_core::gateway::agent_chains::AgentChains;
use wonton_core::gateway::{self, Gateway};
use wonton_core::http::HttpClient;
use wonton_core::matrix::{FederationBlocks, FederationConfig, Matrix, NoticeRoom};
use wonton_core::mcp;
use wonton_core::mcp::audit::Audit;
use wonton_core::members::Members;
use wonton_core::oidc::Hydra;
use wonton_core::owner::OwnerStore;
use wonton_core::pairing::OwnerLanguages;
use wonton_core::peers;
use wonton_core::plugin::PluginManager;
use wonton_core::plugin::docker::Engine;
use wonton_core::plugin::egress::check;
use wonton_core::plugin::egress::settings::Egress;
use wonton_core::plugin::gpu::HostPaths;
use wonton_core::plugin::hostdir::Owner;
use wonton_core::plugin::image_worker::{self, ImageWorker};
use wonton_core::plugin::lan::{self, Lan};
use wonton_core::plugin::languages::Languages;
use wonton_core::plugin::lifecycle::LifecycleConfig;
use wonton_core::plugin::names::Instance;
use wonton_core::plugin::preinstall::Backoff;
use wonton_core::plugin::store::Store;
use wonton_core::push::Push;
use wonton_core::roles::Roles;
use wonton_core::secret;
use wonton_core::tunnel::{self, Tunnel, TunnelEntry};
use wonton_core::upgrade::compose_images;
use wonton_core::upgrade::docker_host::DockerHost;
use wonton_core::upgrade::service::{UpgradeConfig, Upgrades};
use wonton_tunnel::federation::{FEDERATION_ALPN, FederationForwarder, FederationProtocol};
use wonton_tunnel::peer_pair::{PEER_PAIR_ALPN, PeerPairProtocol};

/// 联邦转发组件对每台对方服务器的探活周期和超时。Tuwunel 的联邦请求是并发的，断了的连接靠下一次
/// 使用或探活重连（ADR-0050）。
const FEDERATION_PING_INTERVAL: Duration = Duration::from_secs(30);
const FEDERATION_PING_TIMEOUT: Duration = Duration::from_secs(10);

/// 服务器配对与联邦转发（ADR-0050）：没接 Matrix 的核心返回三个空。
///
/// 返回的转发器由调用方持有到核心退出：`FederationForwarder` 一停，出站转发和探活就全停了。
#[allow(clippy::type_complexity)]
async fn open_peers_and_federation(
    cfg: &Config,
    owner: &Arc<OwnerStore>,
    tunnel: &Tunnel,
    notices: &Option<Arc<NoticeRoom>>,
    manager: &Arc<PluginManager>,
) -> anyhow::Result<(
    Option<Arc<peers::Peers>>,
    Option<FederationProtocol>,
    Option<FederationForwarder>,
)> {
    let Some(matrix) = &cfg.matrix else {
        return Ok((None, None, None));
    };
    let owner_display_name: Arc<dyn Fn() -> String + Send + Sync> = {
        let owner = Arc::clone(owner);
        Arc::new(move || owner.owner().display_name)
    };
    let peers = peers::Peers::open(
        cfg.core_state_dir().join("peers.json"),
        peers::OwnIdentity {
            server_name: matrix.server_name.clone(),
            owner_user_id: format!("@{}:{}", cfg.username, matrix.server_name),
        },
        tunnel.clone(),
        notices
            .clone()
            .map(|notices| notices as Arc<dyn peers::OwnerNotices>),
        owner_display_name,
        {
            let manager = manager.clone();
            Arc::new(move || manager.owner_languages())
        },
    )
    .context("读取服务器配对记录")?;
    // 联邦转发：入站协议挂在隧道的 endpoint 上，出站是给 Tuwunel 用的 CONNECT 代理。
    let upstream = federation_addrs::upstream(cfg).await?;
    let protocol = FederationProtocol::new(peers.server_allowlist().clone(), upstream);
    let lookup = peers.clone();
    let listen = federation_addrs::listen(&cfg.federation_listen).await?;
    let forwarder = FederationForwarder::bind(
        tunnel.endpoint.clone(),
        listen,
        Arc::new(move |name: &str| sharing_start::paired_target(&lookup, name)),
        peers.paired_names(),
        FEDERATION_PING_INTERVAL,
        FEDERATION_PING_TIMEOUT,
    )
    .await
    .with_context(|| format!("联邦转发组件监听 {}", cfg.federation_listen))?;
    tracing::info!(
        listen = %forwarder.listen_addr(),
        upstream = %upstream,
        "联邦转发已启动"
    );
    peers.attach_forwarder(forwarder.watch_status());
    Ok((Some(peers), Some(protocol), Some(forwarder)))
}

/// 隧道 endpoint 上除隧道和设备配对外的协议：联邦入站和服务器配对（ADR-0050）。
fn extra_protocols(
    peers: Option<Arc<peers::Peers>>,
    federation: Option<FederationProtocol>,
    sharing: sharing_start::LimitedShares,
) -> Vec<(&'static [u8], Box<dyn iroh::protocol::DynProtocolHandler>)> {
    let mut extra: Vec<(&'static [u8], Box<dyn iroh::protocol::DynProtocolHandler>)> = Vec::new();
    if let (Some(protocol), Some(peers)) = (federation, peers) {
        extra.push((FEDERATION_ALPN, Box::new(protocol) as _));
        extra.push((
            PEER_PAIR_ALPN,
            Box::new(PeerPairProtocol::new(Arc::new(
                peers::pairing::PeerPairing(peers),
            ))) as _,
        ));
    }
    extra.push((wonton_protocol::SHARE_ALPN, Box::new(sharing) as _));
    extra
}

mod federation_addrs;
mod init_command;
mod lifecycle_commands;
mod mailbox_startup;
use mailbox_startup::{open_mailpit, start_mailpit};
mod pair_command;
mod plugin_command;
mod push_listener;
mod sharing_start;

/// 构建它的提交号：开发镜像构建时由 `deploy/dev/core/dev.sh` 传入，直接 `cargo build` 时没有。
const BUILD_COMMIT: &str = match option_env!("WONTON_BUILD_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};

fn main() -> ExitCode {
    // 处理图片的子进程不起 tokio 运行时，也不初始化日志（日志写在标准输出上，会混进结果）。
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some(image_worker::COMMAND) => return image_worker::child(&args[2..]),
        Some(image_worker::HOST_COMMAND) => return image_worker::host(&args[2..]),
        // 检测容器的结果打印在标准输出上，同样不初始化日志。
        Some(lan::probe::COMMAND) => return lan::probe::command(&args[2..]),
        Some(check::COMMAND) => return check::command(),
        Some(compose_images::COMMAND) => return compose_images::command(),
        _ => {}
    }
    run()
}

#[tokio::main]
async fn run() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let command = std::env::args().nth(1);
    if command.as_deref() == Some("plugin") {
        return plugin_command::run(&std::env::args().skip(2).collect::<Vec<_>>()).await;
    }
    let result = match command.as_deref() {
        None | Some("serve") => serve().await,
        Some("healthcheck") => lifecycle_commands::healthcheck().await,
        Some("pair") => pair_command::run(std::env::args().skip(2).collect()).await,
        Some("init") => init_command::init(&std::env::args().skip(2).collect::<Vec<_>>()).await,
        Some("federation-block") => {
            init_command::federation_block(&std::env::args().skip(2).collect::<Vec<_>>())
        }
        Some("updater") => {
            lifecycle_commands::run_updater(&std::env::args().skip(2).collect::<Vec<_>>()).await
        }
        Some(wonton_core::plugin::network_pool_command::COMMAND) => {
            wonton_core::plugin::network_pool_command::run().await
        }
        Some("manual-rollback") => {
            lifecycle_commands::manual_rollback(&std::env::args().skip(2).collect::<Vec<_>>()).await
        }
        Some("--version") => {
            println!("wonton-core {} ({BUILD_COMMIT})", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(anyhow::anyhow!(
            "未知命令 {other}；用法：wonton-core [serve|healthcheck|init|pair|plugin]"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn check_host_dev(cfg: &Config) -> anyhow::Result<()> {
    if let Some(dev) = &cfg.host_dev
        && !dev.is_dir()
    {
        bail!(
            "WONTON_HOST_DEV 指向的 {} 不是目录：核心在容器里时要把宿主的 /dev 只读挂载到这里",
            dev.display()
        );
    }
    Ok(())
}

fn plugin_engine(cfg: &Config) -> anyhow::Result<Engine> {
    let detector = match &cfg.core_container {
        Some(core) => lan::Detector::Container { core: core.clone() },
        None => lan::Detector::Here,
    };
    Ok(Engine::connect()?.with_network_pool(cfg.plugin_network_pool, detector))
}

fn spawn_federation_config(
    engine: &Engine,
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
        engine.clone(),
        cfg.install_dir.clone(),
        cfg.core_container.clone(),
        notices.clone(),
        names,
        server_name,
        blocks,
    );
}

/// 网关监听先建好再装配：核对插件时的一次性服务要连核心，连接先在内核队列等着。
async fn open_listener(
    listen: std::net::SocketAddr,
) -> anyhow::Result<wonton_core::entry_runtime::MainListener> {
    wonton_core::entry_runtime::MainListener::bind(listen)
        .await
        .with_context(|| format!("监听 {listen}"))
}

fn owner_token(cfg: &Config) -> anyhow::Result<String> {
    let path = cfg.core_state_dir().join("owner-token");
    let token = secret::load_or_create(&path)
        .with_context(|| format!("读取主人令牌 {}", path.display()))?;
    tracing::info!(path = %path.display(), "主人令牌已就绪");
    Ok(token)
}

#[allow(clippy::too_many_lines)] // 启动顺序就是一串有先后的接线，拆开会把顺序藏进多个函数。
async fn serve() -> anyhow::Result<()> {
    tracing::info!(commit = %BUILD_COMMIT, "核心启动");
    let cfg = Config::from_env()?.lock_owner()?;
    tracing::info!(owner = %cfg.username, "主人的用户名已核对");
    let owner = OwnerStore::open(&cfg.core_state_dir())?;
    check_host_dev(&cfg)?;
    let engine = plugin_engine(&cfg)?;
    let docker = engine.check_server().await.context("连接 Docker")?;
    tracing::info!(version = %docker.version, arch = %docker.arch, "Docker 已连接");

    let owner_token = owner_token(&cfg)?;
    let members = Arc::new(Members::open(cfg.core_state_dir().join("members.json"))?);
    let roles = Arc::new(Roles::open(cfg.core_state_dir().join("roles.json"))?);
    let mailpit = open_mailpit(&cfg, &engine, members.clone())?;
    // 监听先建起来：核心启动时核对插件，渲染变了会重建容器，里面的一次性服务（比如登记 OIDC 源）要经核心
    // 的地址访问核心。网关这时还没开始接，连接先进内核的队列等着。
    let listener = open_listener(cfg.listen).await?;

    let upgrades = open_upgrades(&cfg, &engine);
    let sharing_registries = sharing_start::Registries::open(&cfg)?;
    let (http, system_http, hydra, matrix) =
        serve::open_clients(&cfg, &engine, &sharing_registries).await?;
    let hydra_for_manager = hydra.as_ref().map(|(h, _)| h.clone());
    let (manager, images) = start_plugin_manager(
        &cfg,
        engine.clone(),
        upgrades.clone(),
        &http,
        hydra_for_manager,
        &matrix,
        owner.clone(),
        members.clone(),
        roles.clone(),
        &sharing_registries,
        mailpit.clone(),
    )
    .await?;
    let (friends, federation_blocks) = serve::open_friends(&cfg)?;
    let audit = Arc::new(Audit::new(cfg.core_state_dir().join("audit.jsonl")));
    let notices = notice_room(&cfg, matrix.as_ref(), owner_languages(&manager));
    // 隧道先起：确认卡片的开卡推送要用推送登记表和 endpoint ID（ADR-0016）。
    let tunnel = bind_tunnel(&cfg, &manager, members.clone()).await?;
    let push = Arc::new(open_push(&cfg, &tunnel, &manager)?);
    // Bot 调用的确认卡片（ADR-0016）。
    let bus = mcp_bus(&cfg, manager.clone(), audit.clone(), push.clone());
    let server_name = cfg
        .matrix
        .as_ref()
        .map_or_else(|| cfg.public.base_domain.clone(), |m| m.server_name.clone());
    attach_identity(
        &bus,
        matrix.as_ref(),
        &cfg,
        server_name,
        members.clone(),
        roles.clone(),
    );
    bus.attach_help(help_docs(&cfg, manager.clone(), upgrades.clone()));
    let host = Arc::new(
        CoreHost::new(manager.clone(), bus.upstreams(), notices.clone()).with_people(
            members.clone(),
            roles.clone(),
            tunnel.devices.clone(),
            cfg.username.clone(),
            matrix.clone(),
            cfg.core_state_dir(),
        ),
    );
    let files = serve::open_files(&cfg, host, http.clone(), audit.clone(), images)?;
    bus.attach_files(files.clone());
    files.spawn_sweeper();
    // 服务器配对与联邦转发（ADR-0050）：没接 Matrix 的核心没有它们。
    let (peers, federation, _forwarder) =
        open_peers_and_federation(&cfg, &owner, &tunnel, &notices, &manager).await?;
    let forwarder = _forwarder.map(Arc::new);
    let sharing = sharing_registries.service(peers.clone(), forwarder.clone())?;
    // Tuwunel 的名单和 hosts 跟着配对走（ADR-0050）。
    spawn_federation_config(&engine, &cfg, &peers, &notices, federation_blocks.clone());
    let clients = open_clients(&cfg, &engine, &notices)?;
    spawn_entry_runtime(&cfg, &engine, &clients, &notices);
    let contacts = serve::open_contacts(
        &cfg,
        (&manager, &members, &owner),
        Some(push.clone()),
        matrix.as_ref(),
        peers.as_ref(),
    )?;
    if let Some(contacts) = &contacts {
        bus.attach_contacts(contacts.clone());
    }
    // 调用链（ADR-0061 第 10 节）：账本、出箱和设置接上总线和网关。上限卡片从这里发。
    let agent_chains = open_agent_chains(&cfg, &bus)?;
    bus.attach_agent_chains(agent_chains.clone());
    let upgrades_after_serve = upgrades.clone();
    let gateway = open_gateway(
        &cfg,
        owner,
        owner_token,
        members,
        roles,
        manager.clone(),
        hydra,
        http,
        tunnel.clone(),
        system_http.clone(),
        matrix,
        push,
        bus,
        notices,
        files,
        audit,
        upgrades,
        peers.clone(),
        friends,
        federation_blocks,
        sharing,
        contacts,
        clients,
        agent_chains,
    );
    let _mail_worker = start_mailpit(&cfg, mailpit.as_ref(), system_http, gateway.clone())?;
    serve::run_gateway(
        &cfg,
        listener,
        tunnel,
        (peers, federation),
        gateway,
        (manager, upgrades_after_serve),
    )
    .await
}

/// 把网关装起来：确认卡片、文件、配对这些部件都接上（ADR-0004）。
#[allow(clippy::too_many_arguments)]
fn open_gateway(
    cfg: &Config,
    owner: Arc<OwnerStore>,
    owner_token: String,
    members: Arc<Members>,
    roles: Arc<Roles>,
    manager: Arc<PluginManager>,
    hydra: Option<(Hydra, String)>,
    http: HttpClient,
    tunnel: Tunnel,
    system_http: HttpClient,
    matrix: Option<Arc<Matrix>>,
    push: Arc<Push>,
    bus: Arc<mcp::Bus>,
    notices: Option<Arc<NoticeRoom>>,
    files: Arc<Files>,
    audit: Arc<Audit>,
    upgrades: Arc<Upgrades>,
    peers: Option<Arc<peers::Peers>>,
    friends: Arc<Friends>,
    federation_blocks: Option<Arc<FederationBlocks>>,
    sharing: Arc<wonton_core::gateway::sharing::Sharing>,
    contacts: Option<Arc<wonton_core::contacts::Contacts>>,
    clients: Option<Arc<wonton_core::clients::domain::Domain>>,
    agent_chains: Arc<AgentChains>,
) -> Arc<Gateway> {
    let agent_acp = gateway::agent_acp::Registry::new();
    // 停链对 agent_acp 会话的取消（ADR-0061 第 10 节）走这张注册表。
    agent_chains.attach_acp_cancel(agent_acp.clone());
    let gateway = Arc::new(
        Gateway::new(
            cfg.public.clone(),
            owner,
            owner_token,
            members,
            manager,
            hydra,
            http,
            tunnel,
        )
        .with_roles(roles)
        .with_system_http(system_http)
        .with_matrix(matrix)
        .with_send_limiter(Arc::new(gateway::SendLimiter::new(cfg.send_limits.clone())))
        .with_people_limiter(Arc::new(gateway::people_limits::PeopleLimiter::new(
            cfg.people_limits,
        )))
        .with_push(Some(push))
        .with_mcp(Some(bus.endpoint()))
        .with_notices(notices)
        .with_files(Some(files))
        .with_audit(Some(audit))
        .with_tusd(cfg.tusd.clone())
        .with_upgrades(Some(upgrades))
        .with_peers(peers)
        .with_friends(Some(friends))
        .with_contacts(contacts)
        .with_agent_chains(Some(agent_chains))
        .with_agent_acp(Some(agent_acp))
        .with_federation_blocks(federation_blocks)
        .with_sharing(Some(sharing))
        .with_clients(clients),
    );
    if let Some(mcp) = &gateway.mcp {
        mcp.bus.attach_handoff_gateway(Arc::downgrade(&gateway));
    }
    gateway
}

fn lifecycle_config(cfg: &Config, owner: Arc<OwnerStore>) -> LifecycleConfig {
    LifecycleConfig {
        public: cfg.public.clone(),
        data_dir: cfg.data_dir.clone(),
        apps_dir: cfg.apps_dir(),
        shares_dir: cfg.shares_dir(),
        relay_dir: cfg.relay_dir.clone(),
        core_container: cfg.core_container.clone(),
        tz: cfg.tz.clone(),
        puid: cfg.puid,
        pgid: cfg.pgid,
        install_timeout: cfg.install_timeout,
        owner: cfg.username.clone(),
        profile: owner,
        gateway_port: cfg.listen.port(),
        plugin_slice: cfg.plugin_slice.clone(),
        instance: cfg.instance.clone(),
    }
}

fn upgrade_config(cfg: &Config) -> UpgradeConfig {
    UpgradeConfig {
        source: cfg.core_source.clone(),
        install_dir: cfg.install_dir.clone(),
        data_dir: cfg.data_dir.clone(),
        core_container: cfg.core_container.clone(),
        has_instance: cfg.instance != Instance::default(),
    }
}

/// 核心的一键升级（ADR-0025）：渠道来自环境变量，升级动作落到宿主上的 Docker。
fn open_upgrades(cfg: &Config, engine: &Engine) -> Arc<Upgrades> {
    Arc::new(Upgrades::new(
        upgrade_config(cfg),
        Arc::new(DockerHost::new(
            engine.clone(),
            Box::new({
                let data_dir = cfg.data_dir.clone();
                move || wonton_core::plugin::store::read_owner_languages(&data_dir)
            }),
        )),
    ))
}

async fn connect_hydra(cfg: &Config, http: &HttpClient) -> anyhow::Result<Option<(Hydra, String)>> {
    let Some(oidc) = &cfg.oidc else {
        tracing::warn!("没有配置身份提供方，声明 sso: oidc 的插件将无法安装");
        return Ok(None);
    };
    let hydra = Hydra::new(http.clone(), oidc);
    hydra
        .wait_ready(Duration::from_secs(90))
        .await
        .context("等待身份提供方就绪")?;
    tracing::info!(admin = %oidc.admin_url, "身份提供方已就绪");
    Ok(Some((hydra, oidc.public_url.clone())))
}

/// 等 Tuwunel 就绪、准备好核心的管理账号。连不上就启动失败：配置了 Matrix 却用不了，不降级运行。
async fn connect_matrix(cfg: &Config, http: &HttpClient) -> anyhow::Result<Option<Arc<Matrix>>> {
    let Some(config) = &cfg.matrix else {
        tracing::warn!("没有配置 Matrix（WONTON_MATRIX_URL），App 登录 Matrix 和 Bot 插件都不可用");
        return Ok(None);
    };
    let matrix = Matrix::connect(config, &cfg.core_state_dir(), http.clone())
        .await
        .context("连接 Tuwunel")?;
    tracing::info!(
        url = %config.url,
        server_name = %matrix.server_name(),
        admin = %matrix.admin_user_id(),
        "Tuwunel 已就绪"
    );
    Ok(Some(Arc::new(matrix)))
}

/// 在后台预装（要拉镜像、等健康，不挡着网关启动）：先装上还没装的系统插件，再为模型网关的使用方对一次账（凭据的
/// 值在核心，网关的数据丢了也能补回来），最后装默认插件：默认 Agent 要用模型网关。目录配了却不是目录时启动失败。
/// 核心升级没结束时都先等着（ADR-0025）：回滚后插件的记录和容器要对得上。
fn preinstall(
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
        if let Some(dir) = system
            && let Err(e) = manager.ensure_system(&dir, backoff).await
        {
            tracing::error!(error = %e, "预装系统插件失败");
        }
        for (plugin, error) in manager.reconcile_models().await {
            tracing::error!(%plugin, %error, "模型网关凭据对账失败");
        }
        if let Some(dir) = defaults
            && let Err(e) = manager.ensure_defaults(&dir, backoff).await
        {
            tracing::error!(error = %e, "预装默认插件失败");
        }
    });
    Ok(())
}

/// 主人的语言跟着管理器走：App 里改了语言，卡片和通知跟着换。
fn owner_languages(manager: &Arc<PluginManager>) -> Arc<dyn Fn() -> Languages + Send + Sync> {
    let manager = manager.clone();
    Arc::new(move || manager.owner_languages())
}

/// 和主人的通知房间（ADR-0016）：插件通知发进这里，确认卡片不进（ADR-0016 第 6 条）。没接 Matrix 时插件通知如实报不可用。
fn notice_room(
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
fn open_plugin_manager(
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
    .with_host(HostPaths::new(cfg.host_dev.clone()))
    .with_lan(Lan::from_config(cfg))
    .with_egress(
        Egress::new(cfg.egress_proxy.clone(), cfg.egress_proxy_detected.clone())
            .with_doh(cfg.doh_url.clone()),
    )
    .with_store(cfg.store.clone())
    .with_upgrade_freeze(upgrades.frozen())
}

/// 把总线认人用的资料和 Matrix 接上：家人记录、服务器名，以及 Bot 钉页卡片的发送方。
fn attach_identity(
    bus: &Arc<mcp::Bus>,
    matrix: Option<&Arc<Matrix>>,
    cfg: &Config,
    server_name: String,
    members: Arc<Members>,
    roles: Arc<Roles>,
) {
    bus.attach_people(server_name, members, roles);
    bus.attach_matrix(matrix.cloned(), cfg.username.clone());
}

/// MCP 总线（ADR-0016）：开卡时给收件人的设备发推送，之后状态变化只改确认表，设备从
/// “等你确认”列表的长轮询里知道。卡片不进任何 Matrix 房间。
fn mcp_bus(
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
fn open_agent_chains(cfg: &Config, bus: &Arc<mcp::Bus>) -> anyhow::Result<Arc<AgentChains>> {
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
fn message_settled_audit(
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

/// 用安装脚本装的核心（有安装目录和自己的容器名）才管入口和自己的域名。
fn open_clients(
    cfg: &Config,
    engine: &Engine,
    notices: &Option<Arc<NoticeRoom>>,
) -> anyhow::Result<Option<Arc<wonton_core::clients::domain::Domain>>> {
    let (Some(dir), Some(container)) = (&cfg.install_dir, &cfg.core_container) else {
        return Ok(None);
    };
    Ok(Some(wonton_core::clients::domain::Domain::new(
        dir.clone(),
        cfg.data_dir.clone(),
        engine.clone(),
        container.clone(),
        cfg.instance.clone(),
        notices.clone(),
    )))
}

/// 用安装脚本装的核心（有安装目录）才管入口；核心不在容器里时由入口自己报 `core_not_in_container`。
fn spawn_entry_runtime(
    cfg: &Config,
    engine: &Engine,
    clients: &Option<Arc<wonton_core::clients::domain::Domain>>,
    notices: &Option<Arc<NoticeRoom>>,
) {
    let (Some(dir), Some(clients)) = (&cfg.install_dir, clients) else {
        return;
    };
    let containers = wonton_core::entry_containers::EntryContainers::new(
        engine.clone(),
        dir.clone(),
        cfg.data_dir.clone(),
        cfg.instance.clone(),
        cfg.core_container.clone(),
    );
    wonton_core::entry_runtime::spawn(containers, clients.clone(), notices.clone());
}

fn open_push(cfg: &Config, tunnel: &Tunnel, manager: &PluginManager) -> anyhow::Result<Push> {
    let path = cfg.core_state_dir().join("push.json");
    Push::open(&path, tunnel.devices.clone(), tunnel.endpoint.id())
        .and_then(|push| push.with_proxy(manager.core_proxy()))
        .with_context(|| format!("打开推送登记表 {}", path.display()))
}

/// 帮助文档（ADR-0034）。版本号和升级接口报的是同一个。
fn help_docs(
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
async fn start_plugin_manager(
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
    manager.attach_person_state(members, roles);
    manager.attach_sharing(sharing.friends.clone(), sharing.shares.clone());
    manager.spawn_registry_mirror_probe();
    // 启动时把宿主主机名取好缓存起来，装插件、重建容器时不再查 Docker；没取到时下次安装再取。
    manager.host_name().await;
    Ok((manager, images))
}

/// 网关开始接连接之后才做：核对已装插件、起监控、装默认插件。
async fn start_restored_plugins(
    cfg: &Config,
    manager: &Arc<PluginManager>,
    notices: Option<Arc<NoticeRoom>>,
    upgrades: Arc<Upgrades>,
) -> anyhow::Result<()> {
    manager.restore().await.context("恢复已安装的插件")?;
    serve::spawn_plugin_monitors(manager, notices);
    upgrades.spawn();
    preinstall(cfg, manager.clone(), upgrades)?;
    Ok(())
}

/// 读取已配对设备和隧道私钥，绑定隧道 endpoint。绑不上就启动失败：没有隧道，App 连不回家。
async fn bind_tunnel(
    cfg: &Config,
    manager: &Arc<PluginManager>,
    members: Arc<Members>,
) -> anyhow::Result<Tunnel> {
    let devices = Devices::open(cfg.core_state_dir().join("devices.json"))?;
    members.recover_pending_devices(&devices)?;
    let key = tunnel::load_or_create_key(&cfg.core_state_dir().join("tunnel-key"))?;
    let endpoint = tunnel::bind(key, cfg.tunnel_port, &cfg.relays).await?;
    tracing::info!(
        endpoint_id = %endpoint.id(),
        sockets = ?endpoint.bound_sockets(),
        devices = devices.list().len(),
        "隧道入口已启动"
    );
    let addresses = manager.lan_addresses();
    let owner_languages: OwnerLanguages = {
        let manager = manager.clone();
        Arc::new(move || manager.owner_languages())
    };
    let tunnel = Tunnel::new(
        Arc::new(devices),
        members,
        owner_languages,
        endpoint,
        cfg.tunnel_port,
        cfg.relays.clone(),
        addresses.clone(),
    );
    tunnel::spawn_announcing(tunnel.endpoint.clone(), tunnel.port, addresses);
    Ok(tunnel)
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("无法注册 SIGTERM 处理");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
    tracing::info!("收到退出信号");
}
