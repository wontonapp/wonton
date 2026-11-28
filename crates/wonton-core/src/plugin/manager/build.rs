//! 插件管理器的构造：接上 Docker、记录和各个外部入口，状态全部从空开始。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use super::super::docker::Engine;
use super::super::egress::core_proxy::CoreProxy;
use super::super::egress::settings::Egress;
use super::super::gallery::Gallery;
use super::super::github::GitHub;
use super::super::gpu::HostPaths;
use super::super::host_facts::NvidiaMemory;
use super::super::image_worker::ImageWorker;
use super::super::lan::Lan;
use super::super::lifecycle::LifecycleConfig;
use super::super::sources::Sources;
use super::super::staged::StagedUploads;
use super::super::store::Store;
use super::super::storefront::{StoreConfig, Storefront};
use super::super::upgrade::Upgrades;
use super::PluginManager;
use crate::http::HttpClient;
use crate::oidc::Hydra;

impl PluginManager {
    pub fn new(
        engine: Engine,
        store: Store,
        http: HttpClient,
        oidc: Option<Hydra>,
        cfg: LifecycleConfig,
        images: impl Into<std::sync::Arc<ImageWorker>>,
    ) -> Self {
        // 核心直接跑在宿主上时 /dev 就是宿主的；在容器里要由 compose 挂载进来，见 [`Self::with_host`]。
        let host_dev = cfg.core_container.is_none().then(|| PathBuf::from("/dev"));
        // Docker 层的错误按主人最近一次请求的语言写：Engine 自己读不了 store，接上数据目录。
        let mut engine = engine;
        engine.set_languages(store.people_languages(), cfg.owner.clone());
        let lan = Lan::for_core(cfg.core_container.clone());
        let core_proxy = CoreProxy::default();
        let github = GitHub::new(&core_proxy);
        let storefront =
            Storefront::new(github.clone(), StoreConfig::default(), store.index_cache());
        let dependencies =
            super::super::dependencies::Dependencies::open(store.capabilities_path());
        let clients = Arc::new(crate::clients::state::State::new(&store, &cfg.public));
        let client_passwords = match crate::clients::passwords::PasswordStore::open(&cfg.data_dir) {
            Ok(passwords) => Some(Arc::new(passwords)),
            // 打开失败时客户端列表和开关都会报密码库不可用；留下原因，不然只能看到那句报错。
            Err(error) => {
                tracing::error!(%error, "客户端密码库打开失败");
                None
            }
        };
        let client_guests = match crate::clients::guests::GuestStore::open(&cfg.data_dir) {
            Ok(guests) => Some(Arc::new(guests)),
            Err(error) => {
                tracing::error!(%error, "访客链接库打开失败");
                None
            }
        };
        let credential_exec: Arc<dyn crate::clients::credentials::CredentialExec> = Arc::new(
            crate::clients::credentials::EngineExec::new(Arc::new(engine.clone())),
        );
        Self {
            client_passwords,
            client_guests,
            credential_exec: RwLock::new(credential_exec),
            credential_locks: Default::default(),
            engine,
            store,
            http,
            oidc,
            matrix: None,
            matrix_server_name: None,
            matrix_rtc_ca: None,
            mailpit: None,
            profiles: Default::default(),
            host: HostPaths::new(host_dev, None),
            nvidia_memory: NvidiaMemory::default(),
            cfg,
            sources: Sources::default(),
            github,
            storefront,
            core_proxy,
            bot_allocation: tokio::sync::Mutex::new(()),
            bot_web_routes: Default::default(),
            gallery: Gallery::new(images),
            plugins: RwLock::new(BTreeMap::new()),
            failed_installs: RwLock::new(BTreeMap::new()),
            dependencies,
            changes: tokio::sync::watch::Sender::new(0),
            clients,
            clients_gateway: std::sync::OnceLock::new(),
            activity: Arc::default(),
            bundled: RwLock::new(BTreeSet::new()),
            default_bot_plugin: RwLock::new(None),
            lan,
            #[cfg(test)]
            recovery_metadata_hold: Default::default(),
            egress: Egress::default(),
            host_name: tokio::sync::OnceCell::new(),
            registry_mirror: Default::default(),
            upgrade_frozen: None,
            oom: Default::default(),
            upgrades: Upgrades::default(),
            devices: Default::default(),
            staged: StagedUploads::default(),
            people_writes: std::sync::Mutex::new(()),
            sharing: std::sync::OnceLock::new(),
            person_members: std::sync::OnceLock::new(),
            person_roles: RwLock::new(Arc::new(crate::roles::Roles::initial())),
            contact_source: std::sync::OnceLock::new(),
            restoring: std::sync::atomic::AtomicUsize::new(0),
            bot_syncs: RwLock::new(BTreeMap::new()),
        }
    }
}
