//! 安装途中核心重启后，再装能自己恢复；同名的网络不是本插件的就报错、不动它。要真的 Docker Engine：同名时回什么、
//! 按标签能不能找到残留，是 Engine 的行为。
//!
//! 核心重启用丢掉安装任务来扮演：和进程退出一样不会回滚，容器和网络留在 Docker 里，安装记录还没写。核心容器用一个
//! 空转的容器来扮演：它接进插件网络后一直在里面，和真的核心重启后一样，Docker 不让删还有它的网络。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bollard::models::ContainerCreateBody;
use wonton_core::config::PublicUrls;
use wonton_core::http::HttpClient;
use wonton_core::plugin::compose::PullPolicy;
use wonton_core::plugin::docker::{Engine, NetworkReach};
use wonton_core::plugin::image_worker::ImageWorker;
use wonton_core::plugin::lifecycle::LifecycleConfig;
use wonton_core::plugin::manager::PluginManager;
use wonton_core::plugin::names::{Instance, LABEL_PLUGIN};
use wonton_core::plugin::store::Store;

const ID: &str = "app.leftovers";
const SLUG: &str = "app-leftovers";
/// npchy 和 CI 的 runner 连不上 Docker Hub，从 ECR Public 拉。
const IMAGE: &str = "public.ecr.aws/docker/library/busybox:1.37";
/// 收到 TERM 马上退出，删容器时不用等 10 秒。
const PLUGIN: &str = "services:
  web:
    image: public.ecr.aws/docker/library/busybox:1.37
    command: [sh, -c, 'trap \"exit 0\" TERM; sleep 300 & wait']
x-wonton:
  spec: '0.4'
  id: app.leftovers
  version: 1.0.0
  kind: app
  title: {zh: 残留, en: Leftovers}
  description: {zh: 残留, en: Leftovers}
  permissions: []
";

/// 一套用自己实例名的核心：只碰这个测试建的资源，和同一台 Docker 上别的核心、别的 runner 互不相干。
struct Core {
    engine: Engine,
    instance: Instance,
    dir: tempfile::TempDir,
}

impl Core {
    fn new(tag: &str) -> Self {
        let name = format!("{tag}{}", std::process::id());
        Self {
            engine: Engine::connect().unwrap(),
            instance: Instance::parse(Some(&name)).unwrap(),
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// 连上 Docker，先把镜像拉好（下面等容器跑起来的时间不算拉镜像），再起扮演核心的容器。
    async fn prepare(&self) {
        self.engine.check_server().await.unwrap();
        self.engine
            .ensure_image(IMAGE, PullPolicy::Missing, None)
            .await
            .unwrap();
        self.start_idle(&self.core_container()).await;
    }

    async fn start_idle(&self, name: &str) {
        let body = ContainerCreateBody {
            image: Some(IMAGE.to_owned()),
            cmd: Some(["sleep", "300"].map(str::to_owned).into()),
            ..Default::default()
        };
        self.engine.create_container(name, body).await.unwrap();
        self.engine.start_container(name).await.unwrap();
    }

    /// 另一套没设实例名的核心，也用空转的容器扮演。
    fn other_core(&self) -> String {
        format!("{}-other", self.network())
    }

    /// 名字里没有 `--`，不会和插件的容器重名。
    fn core_container(&self) -> String {
        format!("{}-core", self.network())
    }

    /// 核心（重新）启动后的插件管理器，数据目录不变。
    fn manager(&self) -> Arc<PluginManager> {
        use std::os::unix::fs::MetadataExt;

        let owner = std::fs::metadata(self.dir.path()).unwrap();
        let cfg = LifecycleConfig {
            public: PublicUrls {
                scheme: "http".into(),
                base_domain: "wonton.localhost".into(),
                port: 8080,
            },
            data_dir: self.dir.path().to_owned(),
            apps_dir: self.dir.path().join("apps"),
            shares_dir: self.dir.path().join("shares"),
            core_container: Some(self.core_container()),
            tz: "UTC".into(),
            puid: owner.uid(),
            pgid: owner.gid(),
            install_timeout: Duration::from_secs(300),
            owner: "owner".into(),
            profile: {
                let state = self.dir.path().join("owner");
                wonton_core::owner::lock_username(&state, None).unwrap();
                wonton_core::owner::OwnerStore::open(&state).unwrap()
            },
            gateway_port: 8080,
            plugin_slice: None,
            instance: self.instance.clone(),
        };
        let store = Store::open(self.dir.path().join("plugins")).unwrap();
        let images = ImageWorker::new(self.dir.path().join("no-image-worker"));
        Arc::new(PluginManager::new(
            self.engine.clone(),
            store,
            HttpClient::new(),
            None,
            cfg,
            images,
        ))
    }

    fn network(&self) -> String {
        self.instance.network(SLUG)
    }

    fn container(&self) -> String {
        self.instance.container(SLUG, "web")
    }

    async fn container_id(&self) -> Option<String> {
        let inspect = self.engine.inspect_container(&self.container()).await;
        inspect.unwrap().and_then(|c| c.id)
    }

    /// 容器跑起来了就返回它的 ID：这时网络和容器都建好了，安装还在等它稳定运行，没写记录。
    async fn running(&self) -> Option<String> {
        let inspect = self.engine.inspect_container(&self.container()).await;
        let container = inspect.unwrap()?;
        let running = container.state.and_then(|s| s.running) == Some(true);
        running.then(|| container.id.unwrap())
    }

    /// 删掉这个测试建的全部资源。
    async fn clean(&self) {
        for idle in [self.core_container(), self.other_core()] {
            self.engine.remove_container(&idle).await.unwrap();
        }
        let found = self
            .engine
            .labeled(LABEL_PLUGIN, ID, |labels| self.instance.owns(labels))
            .await
            .unwrap();
        for container in &found.containers {
            self.engine.remove_container(container).await.unwrap();
        }
        for network in found.networks {
            self.engine.remove_network(&network).await.unwrap();
        }
        self.engine.remove_network(&self.network()).await.unwrap();
    }
}

/// 装到容器跑起来时核心重启，返回留下的容器的 ID。
async fn restart_midway(core: &Core) -> Result<String, String> {
    let manager = core.manager();
    let install = tokio::spawn(async move { manager.install(PLUGIN, &[]).await });
    let mut orphan = None;
    // 共用的主机很忙时，Docker 建一个容器也可能要一两分钟。
    for _ in 0..3000 {
        if install.is_finished() {
            let done = install.await.map_err(|e| e.to_string())?;
            return Err(format!(
                "安装没等到被打断就结束了：{:?}",
                done.map(|v| v.id)
            ));
        }
        orphan = core.running().await;
        if orphan.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let orphan = orphan.ok_or_else(|| format!("容器 {} 五分钟内没跑起来", core.container()))?;
    install.abort();
    install.await.ok();
    if core.container_id().await.as_ref() != Some(&orphan) {
        return Err("丢掉安装任务后容器应当还在：这才是残留".into());
    }
    Ok(orphan)
}

async fn recovers(core: &Core) -> Result<(), String> {
    let orphan = restart_midway(core).await?;
    let view = core.manager().install(PLUGIN, &[]).await;
    let view = view.map_err(|e| format!("再装失败（{}）：{e}", e.kind()))?;
    if core.container_id().await.as_ref() == Some(&orphan) {
        return Err("容器拿的是上次的配置和令牌，应当重建".into());
    }
    if view.id.as_str() != ID {
        return Err(format!("装上的是 {}", view.id));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn installing_again_recovers_from_a_core_restart_midway_through_an_install() {
    let core = Core::new("lefta");
    core.prepare().await;
    let outcome = recovers(&core).await;
    core.clean().await;
    outcome.unwrap();
}

async fn stays_away(core: &Core) -> Result<(), String> {
    let plugin = restart_midway(core).await?;
    let other = core.other_core();
    core.start_idle(&other).await;
    core.engine
        .connect_network(&core.network(), &other, Vec::new())
        .await
        .map_err(|e| e.to_string())?;
    let Err(error) = core.manager().install(PLUGIN, &[]).await else {
        return Err("另一套核心接在网络上，不应当装上".into());
    };
    if error.kind() != "conflict" || !error.to_string().contains(&other) {
        return Err(format!("应当说出是谁接在上面（{}）：{error}", error.kind()));
    }
    if core.container_id().await.as_ref() != Some(&plugin) {
        return Err("另一套核心的插件容器不能动".into());
    }
    let reach = core.engine.network_reach(&core.network()).await;
    if reach.map_err(|e| e.to_string())?.is_none() {
        return Err("另一套核心的插件网络不能动".into());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_plugin_of_another_running_core_is_left_alone() {
    let core = Core::new("leftc");
    core.prepare().await;
    let outcome = stays_away(&core).await;
    core.clean().await;
    outcome.unwrap();
}

async fn refuses(core: &Core) -> Result<(), String> {
    let network = core.network();
    let labels = HashMap::new();
    core.engine
        .create_network(&network, &labels, NetworkReach::Egress)
        .await
        .map_err(|e| e.to_string())?;
    let Err(error) = core.manager().install(PLUGIN, &[]).await else {
        return Err("同名网络不是本插件的，不应当装上".into());
    };
    if error.kind() != "conflict" || !error.to_string().contains(&network) {
        return Err(format!("应当如实说网络被占（{}）：{error}", error.kind()));
    }
    let reach = core.engine.network_reach(&network).await;
    if reach.map_err(|e| e.to_string())? != Some(NetworkReach::Egress) {
        return Err("同名网络应当原样留着".into());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_network_the_plugin_does_not_own_is_reported_and_left_alone() {
    let core = Core::new("leftb");
    core.prepare().await;
    let outcome = refuses(&core).await;
    core.clean().await;
    outcome.unwrap();
}
