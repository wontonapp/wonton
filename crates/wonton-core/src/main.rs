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
use wonton_core::plugin::preinstall::{Backoff, default_bot_plugin_in};
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
    // 本机对外出示的证书（ADR-0112）：装了证书材料就必须读得出来，读不出来按启动失败。
    match std::env::var_os("WONTON_PEER_TLS_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => {
            peers.set_own_certificate(peers::load_own_certificate(std::path::Path::new(&dir))?)
        }
        None => tracing::info!("没有 WONTON_PEER_TLS_DIR，配对不互换证书"),
    }
    // 联邦转发：入站协议挂在隧道的 endpoint 上，出站是给 Tuwunel 用的 CONNECT 代理。上游地址按连接现
    // 解析（入口容器运行时会被对账重建，启动时钉死 SocketAddr 会在重建后断流）。
    let protocol = FederationProtocol::new(
        peers.server_allowlist().clone(),
        cfg.matrix_federation_addr.clone(),
    );
    let lookup = peers.clone();
    let listen = federation_addrs::listen(&cfg.federation_listen).await?;
    let forwarder = FederationForwarder::bind(
        tunnel.endpoint.clone(),
        listen,
        Arc::new(move |name: &str| sharing_start::paired_target(&lookup, name)),
        peers.paired_names(),
        FEDERATION_PING_INTERVAL,
        FEDERATION_PING_TIMEOUT,
        None,
    )
    .await
    .with_context(|| format!("联邦转发组件监听 {}", cfg.federation_listen))?;
    tracing::info!(
        listen = %forwarder.listen_addr(),
        upstream = %cfg.matrix_federation_addr,
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
use wonton_core::build_info;

const BUILD_COMMIT: &str = match build_info::COMMIT {
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
            println!("wonton-core {} ({BUILD_COMMIT})", build_info::version());
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

/// 网关监听先建好再装配：核对插件时的一次性服务要连核心，连接先在内核队列等着。
#[allow(clippy::too_many_lines)] // 启动顺序就是一串有先后的接线，拆开会把顺序藏进多个函数。
async fn serve() -> anyhow::Result<()> {
    tracing::info!(version = build_info::version(), commit = %BUILD_COMMIT, "核心启动");
    let cfg = Config::from_env()?.lock_owner()?;
    tracing::info!(owner = %cfg.username, "主人的用户名已核对");
    let owner = OwnerStore::open(&cfg.core_state_dir())?;
    serve::check_host_dev(&cfg)?;
    let engine = serve::plugin_engine(&cfg)?;
    let docker = engine.check_server().await.context("连接 Docker")?;
    tracing::info!(version = %docker.version, arch = %docker.arch, "Docker 已连接");

    let owner_token = serve::owner_token(&cfg)?;
    let members = Arc::new(Members::open(cfg.core_state_dir().join("members.json"))?);
    let roles = Arc::new(Roles::open(cfg.core_state_dir().join("roles.json"))?);
    let mailpit = open_mailpit(&cfg, &engine, members.clone())?;
    // 监听先建起来：核心启动时核对插件，渲染变了会重建容器，里面的一次性服务（比如登记 OIDC 源）要经核心
    // 的地址访问核心。网关这时还没开始接，连接先进内核的队列等着。
    let listener = serve::open_listener(cfg.listen).await?;

    let upgrades = open_upgrades(&cfg, &engine);
    let sharing_registries = sharing_start::Registries::open(&cfg, roles.clone())?;
    let (http, system_http, hydra, matrix) =
        serve::open_clients(&cfg, &engine, &sharing_registries).await?;
    let hydra_for_manager = hydra.as_ref().map(|(h, _)| h.clone());
    let (manager, images) = serve::start_plugin_manager(
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
    let friends = sharing_registries.friends.clone();
    let federation_blocks = serve::open_federation_blocks(&cfg)?;
    let audit = Arc::new(Audit::new(cfg.core_state_dir().join("audit.jsonl")));
    let notices = serve::notice_room(&cfg, matrix.as_ref(), serve::owner_languages(&manager));
    // 隧道先起：确认卡片的开卡推送要用推送登记表和 endpoint ID（ADR-0016）。
    let tunnel = serve::bind_tunnel(&cfg, &manager, members.clone()).await?;
    let push = Arc::new(serve::open_push(&cfg, &tunnel, &manager)?);
    // Bot 调用的确认卡片（ADR-0016）。
    let bus = serve::mcp_bus(&cfg, manager.clone(), audit.clone(), push.clone());
    let server_name = cfg
        .matrix
        .as_ref()
        .map_or_else(|| cfg.public.base_domain.clone(), |m| m.server_name.clone());
    serve::attach_identity(
        &bus,
        matrix.as_ref(),
        &cfg,
        server_name,
        members.clone(),
        roles.clone(),
        push.clone(),
    );
    bus.attach_help(serve::help_docs(&cfg, manager.clone(), upgrades.clone()));
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
    // 入口容器先对账：把上一版 compose 容器认领掉、按设置把入口建好，主人打开的公网入口和联邦 TLS 前端
    // 才在服务。失败按类别记日志、照常启动（不挡核心），带退避的重试在入口的运行循环里；入站联邦的
    // 上游地址按连接现解析，入口容器不在时那条流连不上、核心照常服务。
    // 公网访问总开关（ADR-0090 第 2 节）：运行循环、网关和配对状态都读它。
    let access = Arc::new(wonton_core::entry_access::EntryAccess::open(
        cfg.core_state_dir().join("entry-access.json"),
    )?);
    let entry = serve::start_entry_runtime(&cfg, &engine, &access).await;
    // 服务器配对与联邦转发（ADR-0050）：没接 Matrix 的核心没有它们。
    let (peers, federation, _forwarder) =
        open_peers_and_federation(&cfg, &owner, &tunnel, &notices, &manager).await?;
    let forwarder = _forwarder.map(Arc::new);
    if let Some(peers) = &peers {
        peers.attach_public_access(access.clone());
    }
    let sharing = sharing_registries.service(peers.clone(), forwarder.clone())?;
    // Tuwunel 的名单和 hosts 跟着配对走（ADR-0050）。
    serve::spawn_federation_config(
        matrix.as_ref(),
        &cfg,
        &peers,
        &notices,
        federation_blocks.clone(),
    );
    let clients = serve::open_domain(&cfg, &engine, &notices)?;
    let entry_admin = cfg.install_dir.as_ref().map(|dir| {
        Arc::new(wonton_core::entry_access::EntryAdmin::new(
            access.clone(),
            entry.clone(),
            dir.clone(),
        ))
    });
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
    let agent_chains = serve::open_agent_chains(&cfg, &bus)?;
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
        notices.clone(),
        files,
        audit,
        upgrades,
        peers.clone(),
        friends,
        federation_blocks,
        sharing,
        contacts,
        clients.clone(),
        agent_chains,
        entry_admin,
        access.public_access_flag(),
    );
    manager.attach_clients_gateway(&gateway);
    if let Err(error) = wonton_core::entry_runtime::reconcile_clients(&gateway).await {
        tracing::warn!(%error, "启动时客户端入口配置未对上，运行循环会重试");
    }
    if let (Some(containers), Some(clients)) = (entry, clients) {
        wonton_core::entry_runtime::spawn(
            containers,
            access.clone(),
            clients,
            notices.clone(),
            Some(gateway.clone()),
        );
    }
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
    entry: Option<Arc<wonton_core::entry_access::EntryAdmin>>,
    public_access: Arc<std::sync::atomic::AtomicBool>,
) -> Arc<Gateway> {
    Arc::new(
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
        .with_federation_blocks(federation_blocks)
        .with_sharing(Some(sharing))
        .with_clients(clients)
        .with_entry(entry)
        .with_public_access_flag(public_access),
    )
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
