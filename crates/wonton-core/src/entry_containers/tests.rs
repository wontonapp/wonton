//! 入口容器和连接器的期望状态：按设置建、删，安全选项逐项锁住。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use super::{
    CONNECTOR_RELOAD_ADDR, ENTRY_CLIENTS_ALIAS, ENTRY_EDGE_ALIAS, ENTRY_RELOAD_ADDR,
    EntryContainers, EntryError,
};
use crate::entry_compose::HAPROXY_IMAGE;
use crate::entry_haproxy::CONNECTOR_ADDRESS;
use crate::entry_runtime::FRPC_IMAGE;
use crate::entry_settings::EntrySettings;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::names::Instance;

const CORE: &str = "wonton-core";
const PROJECT: &str = "wonton";

fn settings() -> EntrySettings {
    EntrySettings {
        domain: "k3f9qzab.example.test".to_owned(),
        service_url: None,
        service_ca: None,
        relay_ca: None,
        registration: None,
        acme_directory: None,
        acme_ca: None,
        acme_resolvers: Vec::new(),
        acme_email: None,
        acme_accept_terms: false,
        acme_network: "bridge".to_owned(),
        dns_provider: None,
        dns_env_file: None,
        acme_propagation_disable_ans: false,
        external_certificate: true,
    }
}

/// 主人在 App 里配了自己的域名：“不要名字”的安装没有设置文件，只有这份域名设置。
fn app_domain(rig: &Rig, suffix: &str) {
    std::fs::write(
        rig.dir.path().join("entry/domain.json"),
        serde_json::to_vec(&json!({"suffix": suffix, "terms_accepted": true})).unwrap(),
    )
    .unwrap();
}

/// 发布一张覆盖这些名字的证书，和签发成功落盘的 `site.pem` 一样。
fn publish_cert(rig: &Rig, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    let cert = rcgen::generate_simple_self_signed(names).unwrap();
    std::fs::write(rig.dir.path().join("entry/certs/site.pem"), cert.cert.pem()).unwrap();
}

/// 命令行 `-f`（或 frpc 的 `-c`）指到的配置文件：容器里的 `/config` 就是 `entry/`。
fn command_file(rig: &Rig, container: &str) -> std::path::PathBuf {
    let created = rig.created(container);
    let arguments = created["Cmd"].as_array().unwrap();
    let position = arguments
        .iter()
        .position(|argument| argument == "-f" || argument == "-c")
        .unwrap();
    let name = arguments[position + 1]
        .as_str()
        .unwrap()
        .strip_prefix("/config/")
        .unwrap();
    rig.dir.path().join("entry").join(name)
}

/// 容器挂进去的每个源路径都得真的在：缺了 Docker 会在宿主侧建成目录，haproxy 起不来。
fn assert_mounts_exist(rig: &Rig, container: &str) {
    let created = rig.created(container);
    for bind in created["HostConfig"]["Binds"].as_array().unwrap() {
        let source = bind.as_str().unwrap().split(':').next().unwrap();
        assert!(std::path::Path::new(source).exists(), "{source} 不在");
    }
}

fn config(rig: &Rig) -> String {
    std::fs::read_to_string(rig.dir.path().join("entry/haproxy.cfg")).unwrap_or_default()
}
/// 公网 crt-list 里列的名字。
fn served_names(rig: &Rig) -> Vec<String> {
    std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list"))
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

struct Rig {
    docker: FakeDocker,
    containers: EntryContainers,
    dir: tempfile::TempDir,
    /// 管理端口被连了几次：每连一次就是一次 reload。
    reloads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Rig {
    /// 核心在自己的 compose 项目里；`wanted` 里的网络先建好（全新安装时 edge、relay 还不存在）。
    async fn new() -> Self {
        Self::with_networks(&["clients", "edge", "relay"]).await
    }

    async fn with_networks(networks: &[&str]) -> Self {
        let (docker, engine) = FakeDocker::start().await;
        docker.add_stranger_container(
            CORE,
            json!({"Labels": {"com.docker.compose.project": PROJECT}}),
        );
        for network in networks {
            docker.add_unlabeled_network(&format!("{PROJECT}_{network}"));
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        let (reload_addr, reloads) = counting_master().await;
        let containers = EntryContainers::new(
            engine,
            dir.path().to_owned(),
            dir.path().join("data"),
            Instance::default(),
            Some(CORE.to_owned()),
        )
        .with_reload_addr(reload_addr);
        Self {
            docker,
            containers,
            dir,
            reloads,
        }
    }

    fn reloads(&self) -> usize {
        self.reloads.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn reconcile(&self, settings: Option<&EntrySettings>) -> Result<(), EntryError> {
        self.containers.reconcile(settings).await
    }

    fn created(&self, name: &str) -> Value {
        self.docker.created(name).unwrap_or_default()
    }

    /// 核心自己也在容器的表里，这里只看它以外的。
    fn others(&self) -> Vec<String> {
        self.docker
            .containers()
            .into_iter()
            .filter(|name| name != CORE)
            .collect()
    }

    fn writes_for(&self, needle: &str) -> Vec<String> {
        self.docker
            .writes()
            .into_iter()
            .filter(|write| write.contains(needle))
            .collect()
    }
}

/// 没有入口设置：两个容器都不在，也不去动网络。
#[tokio::test]
async fn no_entry_settings_no_containers() {
    let rig = Rig::new().await;
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.others(), Vec::<String>::new());
    assert!(rig.writes_for("/networks/").is_empty());
}

/// 有联邦：入口容器和连接器都建起来，接在网络里。
#[tokio::test]
async fn federation_settings_create_entry_and_connector() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let entry = rig.created("wonton-entry");
    let connector = rig.created("wonton-connector");
    assert_eq!(entry["Image"], HAPROXY_IMAGE);
    assert_eq!(
        entry["Cmd"],
        json!([
            "haproxy",
            "-W",
            "-db",
            "-f",
            "/config/haproxy.cfg",
            "-S",
            ENTRY_RELOAD_ADDR
        ])
    );
    assert_eq!(connector["Image"], HAPROXY_IMAGE);
    assert_eq!(
        connector["Cmd"],
        json!([
            "haproxy",
            "-W",
            "-db",
            "-f",
            "/config/connector.cfg",
            "-S",
            CONNECTOR_RELOAD_ADDR
        ])
    );
    // 入口在 edge 和 clients 上，连接器在 edge 和 relay 上。
    let edge = format!("{PROJECT}_edge");
    let clients = format!("{PROJECT}_clients");
    let relay = format!("{PROJECT}_relay");
    assert_eq!(
        rig.docker.endpoint(&clients, "wonton-entry").unwrap()["Aliases"],
        json!([ENTRY_CLIENTS_ALIAS])
    );
    assert_eq!(
        rig.docker.endpoint(&edge, "wonton-entry").unwrap()["Aliases"],
        json!([ENTRY_EDGE_ALIAS])
    );
    assert!(rig.docker.endpoint(&relay, "wonton-entry").is_none());
    assert!(rig.docker.endpoint(&clients, "wonton-connector").is_none());
    assert!(rig.docker.endpoint(&relay, "wonton-connector").is_some());
    assert_eq!(rig.docker.gateway_priority(&edge), Some(-1));
    // 端点写在创建请求里，不走 `network_mode: none` 再 connect（Docker 拒绝那种接法）。
    assert_ne!(entry["HostConfig"]["NetworkMode"], json!("none"));
    assert!(entry["NetworkingConfig"]["EndpointsConfig"][&clients].is_object());
    assert!(entry["NetworkingConfig"]["EndpointsConfig"][&edge].is_object());
}

/// 安全选项逐项断言：非 root、只读根、能力全去、不升权、清代理变量、没有 Docker socket 挂载。
#[tokio::test]
async fn containers_have_security_options() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let user = std::fs::metadata(rig.dir.path().join("entry/certs/site.pem"))
        .map(|meta| {
            use std::os::unix::fs::MetadataExt;
            format!("{}:{}", meta.uid(), meta.gid())
        })
        .unwrap();
    for name in ["wonton-entry", "wonton-connector"] {
        let created = rig.created(name);
        let host = &created["HostConfig"];
        assert_eq!(created["User"], json!(user), "{name} 的用户");
        assert_eq!(host["ReadonlyRootfs"], json!(true), "{name}");
        assert_eq!(host["CapDrop"], json!(["ALL"]), "{name}");
        assert_eq!(
            host["SecurityOpt"],
            json!(["no-new-privileges:true"]),
            "{name}"
        );
        let env: HashMap<&str, &str> = created["Env"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.as_str()?.split_once('='))
            .collect();
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            assert_eq!(env.get(name), Some(&""), "{name}");
        }
        let binds: Vec<&str> = host["Binds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            binds.iter().all(|bind| !bind.contains("docker.sock")),
            "{binds:?}"
        );
        assert_eq!(host["LogConfig"]["Config"]["max-size"], "10m");
        assert_eq!(host["LogConfig"]["Config"]["max-file"], "3");
        // Docker 重启后要自己回来：核心发过信号的容器会被记成手动停过，`unless-stopped` 不再拉起。
        assert_eq!(host["RestartPolicy"]["Name"], "always");
        assert_eq!(created["StopSignal"], "SIGTERM");
    }
    // 连接器不在 core 网络上；入口也不在。
    let core = format!("{PROJECT}_core");
    assert!(rig.docker.endpoint(&core, "wonton-connector").is_none());
    assert!(rig.docker.endpoint(&core, "wonton-entry").is_none());
}

/// 重复对账不重建、不改网络。
#[tokio::test]
async fn reconcile_is_idempotent() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
    assert_eq!(rig.docker.creations("wonton-connector"), 1);
    // 端点都在创建请求里，第二次对账既不重建也不补接网络。
    let connects = rig.writes_for("/connect").len();
    assert_eq!(connects, 0, "{connects}");
}

/// 设置撤了：两个容器都删掉。
#[tokio::test]
async fn removed_settings_remove_containers() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.others(), Vec::<String>::new());
}

/// “不要名字”的安装没有设置文件：主人在 App 里配了自己的域名（`entry/domain.json`）照样要有入口。
/// 这是 #451 的回归：只看设置文件时这里会按“两个都不要”删。
#[tokio::test]
async fn app_domain_without_settings_creates_entry_and_connector() {
    let rig = Rig::new().await;
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
    // 没有家名服务：连接器是路由器转发用的 HAProxy，不是 frpc。
    assert_eq!(rig.created("wonton-connector")["Image"], HAPROXY_IMAGE);
    // 入口配置按这个域名重写。
    assert!(
        config(&rig).contains("home.example.test"),
        "{}",
        config(&rig)
    );
}

/// 全新安装时 `edge` 就是默认网段，算出来的连接器地址和装机时写的一样：配置不能因此不写。
/// 缺文件时 Docker 会在宿主侧把这些路径建成目录，haproxy 在参数解析阶段就退出 1（复审 878 第 1 条）。
/// 对账必须把目录换回文件。
#[tokio::test]
async fn the_default_edge_subnet_still_gets_the_whole_entry_config() {
    let rig = Rig::new().await;
    rig.docker.pin_subnet(
        &format!("{PROJECT}_edge"),
        crate::entry_haproxy::EDGE_SUBNET,
    );
    app_domain(&rig, "home.example.test");
    // 旧版没写文件就建了容器：挂载源已经被 Docker 建成了目录。
    for name in [
        "entry/haproxy.cfg",
        "entry/certs/public.crt-list",
        "entry/certs/lan.crt-list",
        "entry/connector.cfg",
    ] {
        std::fs::create_dir_all(rig.dir.path().join(name)).unwrap();
    }
    rig.reconcile(None).await.unwrap();
    eprintln!(
        "DEBUG writes {:?} containers {:?} others {:?}",
        rig.docker.writes(),
        rig.docker.containers(),
        rig.others()
    );
    let config = config(&rig);
    assert!(!config.is_empty(), "默认网段下入口配置没写出来");
    assert!(
        config.contains(&format!("src {CONNECTOR_ADDRESS}")),
        "{config}"
    );
    assert!(
        std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list"))
            .unwrap()
            .contains("home.example.test")
    );
    assert!(rig.dir.path().join("entry/certs/lan.crt-list").is_file());
    let connector_config = command_file(&rig, "wonton-connector");
    assert!(
        connector_config.is_file(),
        "{} 不在",
        connector_config.display()
    );
    for container in ["wonton-entry", "wonton-connector"] {
        assert_mounts_exist(&rig, container);
    }
}

/// 换了后缀但新证书还没发布：入口继续按旧证书里的名字服务，新证书发布后的下一轮才换、才 reload。
#[tokio::test]
async fn a_changed_domain_keeps_the_published_certificate_untill_a_new_one_exists() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["old.example.test", "probe.old.example.test"]);
    app_domain(&rig, "old.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(config(&rig).contains("old.example.test"));
    let reloads = rig.reloads();
    app_domain(&rig, "new.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(
        config(&rig).contains("old.example.test"),
        "{}",
        config(&rig)
    );
    assert!(
        !config(&rig).contains("new.example.test"),
        "{}",
        config(&rig)
    );
    assert_eq!(rig.reloads(), reloads, "名字没换不该 reload");
    publish_cert(&rig, &["new.example.test", "probe.new.example.test"]);
    rig.reconcile(None).await.unwrap();
    assert!(config(&rig).contains("new.example.test"));
    assert!(!config(&rig).contains("old.example.test"));
    assert_eq!(rig.reloads(), reloads + 1);
}
/// 旧证书只有子域、后缀改成它的父域：父域不在旧证书的覆盖范围里，配置留在旧名字；
/// 新证书覆盖父域发布后的下一轮才换。
#[tokio::test]
async fn a_renamed_parent_domain_keeps_the_subdomain_certificate() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["old.example.test", "probe.old.example.test"]);
    app_domain(&rig, "old.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(served_names(&rig).contains(&"old.example.test".to_owned()));
    let reloads = rig.reloads();
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    assert!(
        served_names(&rig).contains(&"old.example.test".to_owned()),
        "{:?}",
        served_names(&rig)
    );
    assert!(
        !served_names(&rig).contains(&"example.test".to_owned()),
        "{:?}",
        served_names(&rig)
    );
    assert_eq!(rig.reloads(), reloads, "名字没换不该 reload");
    publish_cert(&rig, &["example.test", "*.example.test"]);
    rig.reconcile(None).await.unwrap();
    assert!(served_names(&rig).contains(&"example.test".to_owned()));
    assert!(!served_names(&rig).contains(&"old.example.test".to_owned()));
    assert_eq!(rig.reloads(), reloads + 1);
}

/// 同名续期：配置文本一个字都不变，换的只是 `site.pem` 的内容，也必须让入口重读。
#[tokio::test]
async fn a_same_name_renewal_still_reloads() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    let reloads = rig.reloads();
    // 同名、内容不同的新证书：crt-list 的名字和内容都不变。
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        rig.reloads(),
        reloads + 1,
        "证书内容换了，配置文本没变也必须让入口重读"
    );
}

/// 重载失败又碰上容器重建：失败那轮不能把证书记成已加载，下一轮要再试。
#[tokio::test]
async fn a_failed_reload_with_a_recreated_container_is_retried() {
    let mut rig = Rig::new().await;
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    // 第一条连接拒（带启动日志），第二条起成功。
    let (addr, attempts) = failing_once_master().await;
    rig.containers.reload_addr = addr;
    rig.docker.remove("wonton-connector");
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    let error = rig.reconcile(None).await.unwrap_err();
    assert_eq!(error.category(), "upstream_error");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "重载失败后的下一轮要再试"
    );
}

/// 只有配置变（证书没变）的重载失败，也要在下一轮重试：失败不能算恢复。
#[tokio::test]
async fn a_failed_config_reload_is_retried() {
    let mut rig = Rig::new().await;
    publish_cert(&rig, &["example.test", "*.example.test"]);
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    let (addr, attempts) = failing_once_master().await;
    rig.containers.reload_addr = addr;
    rig.docker.remove("wonton-connector");
    // 证书覆盖新名字，配置按新名字写；证书本身没换。
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap_err();
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "配置变化的重载失败后下一轮要再试"
    );
}

/// 同上，但这一轮没有容器重建：失败的重试和“有没有 started”无关。
#[tokio::test]
async fn a_failed_config_reload_is_retried_without_a_restart() {
    let mut rig = Rig::new().await;
    publish_cert(&rig, &["example.test", "*.example.test"]);
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    let (addr, attempts) = failing_once_master().await;
    rig.containers.reload_addr = addr;
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap_err();
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "配置变化的重载失败也跨轮重试"
    );
}

/// 配置写下去之后、容器动作失败：这笔账不能丢，下一轮要让入口读新配置。
#[tokio::test]
async fn a_write_then_failed_container_action_is_retried() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["example.test", "*.example.test"]);
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    let reloads = rig.reloads();
    app_domain(&rig, "home.example.test");
    rig.docker
        .fail_inspect("wonton-entry", Some("inspect boom"));
    rig.reconcile(None).await.unwrap_err();
    rig.docker.fail_inspect("wonton-entry", None);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        rig.reloads(),
        reloads + 1,
        "配置已经写下去，下一轮必须让入口读"
    );
}

/// 重复对账不重建：每轮都删了再建会让公网入口一直断。
#[tokio::test]
async fn app_domain_without_settings_is_idempotent() {
    let rig = Rig::new().await;
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
    assert_eq!(rig.docker.creations("wonton-connector"), 1);
}

/// 装机时的设置文件没了、App 里配的域名还在：入口容器留着，不按“没有设置”删掉。
#[tokio::test]
async fn app_domain_keeps_containers_after_settings_disappear() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    app_domain(&rig, &settings().domain);
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
    assert_eq!(rig.docker.creations("wonton-connector"), 1);
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
}

/// 域名配好了但证书还没签出来：先不建入口（入口要读 `site.pem` 定运行用户），签出来后的下一轮再建。
#[tokio::test]
async fn app_domain_without_a_certificate_waits_for_the_next_reconcile() {
    let rig = Rig::new().await;
    app_domain(&rig, "home.example.test");
    std::fs::remove_file(rig.dir.path().join("entry/certs/site.pem")).unwrap();
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.others(), Vec::<String>::new());
    std::fs::write(rig.dir.path().join("entry/certs/site.pem"), "pem").unwrap();
    rig.reconcile(None).await.unwrap();
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
}

/// 域名设置读不了是“不确定”：不按没域名删掉在跑的入口，如实报错让循环退避重试。
#[tokio::test]
async fn an_unreadable_app_domain_keeps_the_containers() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    std::fs::write(rig.dir.path().join("entry/domain.json"), "{").unwrap();
    let error = rig.reconcile(None).await.unwrap_err();
    assert_eq!(error.category(), "invalid_input");
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
}

/// 有设置文件时它说了算：名字和家名服务都按设置，域名设置文件里另写的后缀不参与。
#[tokio::test]
async fn settings_win_over_the_app_domain() {
    let rig = Rig::new().await;
    let mut settings = settings();
    settings.registration = Some(federation_registration());
    let ca = rig.dir.path().join("relay-ca.pem");
    std::fs::write(&ca, "ca").unwrap();
    settings.relay_ca = Some(ca);
    app_domain(&rig, "other.example.test");
    rig.reconcile(Some(&settings)).await.unwrap();
    assert_eq!(rig.created("wonton-connector")["Image"], FRPC_IMAGE);
    assert!(config(&rig).contains(&settings.domain));
    assert!(!config(&rig).contains("other.example.test"));
}

/// 核心跑在宿主上：要入口就回 `core_not_in_container`，不建容器。
#[tokio::test]
async fn core_outside_container_reports_core_not_in_container() {
    let rig = Rig::new().await;
    let here = EntryContainers::new(
        rig.docker.engine().await,
        rig.dir.path().to_owned(),
        rig.dir.path().join("data"),
        Instance::default(),
        None,
    );
    let error = here.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "core_not_in_container");
    assert_eq!(
        error.to_string(),
        "core_not_in_container：核心不在容器里，没有入口要的 clients、edge 网络"
    );
    assert_eq!(rig.others(), Vec::<String>::new());
    // 没有设置时是空操作：开发栈没有入口设置不该每次启动都报错。
    here.reconcile(None).await.unwrap();
}

/// 家名服务（中继）时连接器是 frpc，读它的配置和 frps 的 CA。
#[tokio::test]
async fn federation_registration_uses_the_relay_connector() {
    let rig = Rig::new().await;
    let mut settings = settings();
    settings.registration = Some(federation_registration());
    let ca = rig.dir.path().join("relay-ca.pem");
    std::fs::write(&ca, "ca").unwrap();
    settings.relay_ca = Some(ca);
    rig.reconcile(Some(&settings)).await.unwrap();
    let created = rig.created("wonton-connector");
    assert_eq!(created["Image"], FRPC_IMAGE);
    assert_eq!(created["Cmd"], json!(["-c", "/config/frpc.json"]));
    let binds: Vec<&str> = created["HostConfig"]["Binds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        binds.iter().any(|bind| bind.contains("frpc.json")),
        "{binds:?}"
    );
    assert!(
        binds.iter().any(|bind| bind.contains("relay-ca.pem")),
        "{binds:?}"
    );
    assert_eq!(created["HostConfig"]["PortBindings"], Value::Null);
}

/// 路由器转发：连接器发布到宿主（默认只有回环）。
#[tokio::test]
async fn router_forwarding_publishes_the_host_port() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let created = rig.created("wonton-connector");
    assert_eq!(
        created["HostConfig"]["PortBindings"]["8443/tcp"],
        json!([{"HostIp": "127.0.0.1", "HostPort": "8443"}])
    );
}

async fn read_command(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let mut byte = [0u8; 1];
    loop {
        if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
            break;
        }
    }
}

/// 假 HAProxy 主进程的管理端口：收一条命令，回一段固定回应，返回命令原文。
async fn master_cli(reply: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut command = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            socket.read_exact(&mut byte).await.unwrap();
            command.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        socket.write_all(reply.as_bytes()).await.unwrap();
        String::from_utf8(command).unwrap()
    });
    (addr, handle)
}

/// 每次连接都回同一段的假主进程：要在 reconcile 路径上重载的用例用。
/// 数着连接的假主进程：每连一次记一次，回成功。
/// 第一条连接回拒绝（带启动日志），之后回成功；返回连了几次。
async fn failing_once_master() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut byte = [0u8; 1];
                loop {
                    if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
                        break;
                    }
                }
                let first = seen.fetch_add(1, Ordering::SeqCst) == 0;
                let reply: &[u8] = if first {
                    b"Success=0\n--\nconfig error\n"
                } else {
                    b"Success=1\n"
                };
                let _ = socket.write_all(reply).await;
            });
        }
    });
    (addr, attempts)
}

async fn counting_master() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut byte = [0u8; 1];
            loop {
                if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
                    break;
                }
            }
            if socket.write_all(b"Success=1\n").await.is_err() {
                return;
            }
        }
    });
    (addr, count)
}

async fn master_forever(reply: &'static str) -> String {
    use tokio::io::AsyncWriteExt;
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            read_command(&mut socket).await;
            if socket.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
            // 真的 master 返状态后关连接：读的一方靠 EOF 收尾，失败用例也不用干等截止。
            drop(socket);
        }
    });
    addr
}

/// 按顺序回应的假主进程：每次连接取一条回应，返回到管理端口的地址和已收的连接数。
async fn master_sequence(replies: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        for reply in replies {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            read_command(&mut socket).await;
            counter.fetch_add(1, Ordering::SeqCst);
            if socket.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
            drop(socket);
        }
    });
    (addr, hits)
}

/// 第一条连接读了命令后既不回也不关，后续连接按顺序回 `replies`。
async fn master_silent_then(replies: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        let mut silent = Vec::new();
        for index in 0..=replies.len() {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            read_command(&mut socket).await;
            counter.fetch_add(1, Ordering::SeqCst);
            if index == 0 {
                // 不说话也不关，读完命令就搁着。
                silent.push(socket);
                continue;
            }
            if socket
                .write_all(replies[index - 1].as_bytes())
                .await
                .is_err()
            {
                return;
            }
            drop(socket);
        }
    });
    (addr, hits)
}

async fn tokio_listener() -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap()
}

/// 证书续期后向入口 HAProxy 的主进程发 `reload`：按组件标签找，不经 Docker 的 kill 接口。
#[tokio::test]
async fn reload_asks_the_entry_to_soft_reload() {
    let (addr, seen) = master_cli("Success=1\n").await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("没有连管理端口")
        .unwrap();
    assert_eq!(seen, "reload\n");
    assert!(
        rig.writes_for("/kill").is_empty(),
        "{:?}",
        rig.docker.writes()
    );
}

/// 新配置没起来：`Success=0` 后面跟着启动日志，原样报出来。
#[tokio::test]
async fn reload_reports_the_startup_logs_of_a_failed_reload() {
    let (addr, _seen) =
        master_cli("Success=0\n--\n[ALERT] (49) : config : missing timeouts for proxy 'public'.\n")
            .await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let error = rig.containers.reload(&addr).await.unwrap_err();
    let text = error.to_string();
    assert!(text.contains("missing timeouts"), "{text}");
}

/// 容器刚起来：主进程还在初始装载，第一次回“还在重载”，隔一会儿重发后成功。
#[tokio::test]
async fn reload_retries_while_another_reload_is_in_progress() {
    let (addr, hits) = master_sequence(vec![
        "Success=0\n--\nAnother reload is still in progress.\n",
        "Success=1\n",
    ])
    .await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

/// 主进程接了就断、还没开始回应：也算可重试。
#[tokio::test]
async fn reload_retries_a_connection_closed_without_a_reply() {
    let (addr, hits) = master_sequence(vec!["", "Success=1\n"]).await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

/// 连上却一个字都不回：不能吃光 10 秒预算，隔一小段换连接重发。
#[tokio::test]
async fn reload_retries_a_silent_connection() {
    let (addr, hits) = master_silent_then(vec!["Success=1\n"]).await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

/// 主进程只回了半截 `Success=0\n` 就断：不是拒绝，换条连接重发。
#[tokio::test]
async fn reload_retries_a_truncated_rejection() {
    let (addr, hits) = master_sequence(vec!["Success=0\n", "Success=1\n"]).await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn open_lan_writes_lan_crt_list_and_reloads() {
    let rig = Rig::new().await;
    let containers = rig.containers.clone().with_lan_forward_available(true);
    containers
        .apply_clients_model(
            &settings(),
            true,
            vec!["public-app".into()],
            vec!["lan-app".into()],
        )
        .await
        .unwrap();
    let lan = std::fs::read_to_string(rig.dir.path().join("entry/certs/lan.crt-list")).unwrap();
    let public =
        std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list")).unwrap();
    assert!(lan.contains("lan-app.lan.k3f9qzab.example.test"), "{lan}");
    assert!(
        public.contains("public-app.k3f9qzab.example.test"),
        "{public}"
    );
    assert!(rig.writes_for("/kill").is_empty());
}

#[tokio::test]
async fn public_model_respects_public_access_gate() {
    let rig = Rig::new().await;
    rig.containers
        .apply_clients_model(&settings(), false, vec!["app".into()], vec![])
        .await
        .unwrap();
    let path = rig.dir.path().join("entry/certs/public.crt-list");
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("app.k3f9qzab.example.test")
    );
    rig.containers
        .apply_clients_model(&settings(), true, vec!["app".into()], vec![])
        .await
        .unwrap();
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("app.k3f9qzab.example.test")
    );
}

#[tokio::test]
async fn reload_failure_restores_all_configuration() {
    let mut rig = Rig::new().await;
    rig.containers
        .apply_clients_model(&settings(), true, vec!["old".into()], vec![])
        .await
        .unwrap();
    let entry = rig.dir.path().join("entry");
    let paths = ["haproxy.cfg", "certs/public.crt-list", "certs/lan.crt-list"];
    let before: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(entry.join(path)).unwrap())
        .collect();
    rig.containers.reload_addr = master_forever("Success=0\n--\nUSR2 failed\n").await;
    let error = rig
        .containers
        .apply_clients_model(&settings(), true, vec!["new".into()], vec![])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("USR2 failed"), "{error}");
    for (path, expected) in paths.iter().zip(before) {
        assert_eq!(std::fs::read(entry.join(path)).unwrap(), expected, "{path}");
    }
    assert!(rig.writes_for("/kill").is_empty());
}

#[tokio::test]
async fn reconcile_does_not_replace_client_crt_lists() {
    let rig = Rig::new().await;
    rig.containers
        .apply_clients_model(&settings(), true, vec!["app".into()], vec![])
        .await
        .unwrap();
    let path = rig.dir.path().join("entry/certs/public.crt-list");
    let before = std::fs::read(&path).unwrap();
    rig.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before);
}

/// 同名容器是别人建的：不动它，如实报冲突。
#[tokio::test]
async fn a_stranger_with_the_name_is_reported() {
    let rig = Rig::new().await;
    rig.docker
        .add_stranger_container("wonton-entry", json!({"Image": "other:1"}));
    let error = rig.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "conflict");
    assert!(error.to_string().contains("wonton-entry"), "{error}");
    assert_eq!(rig.created("wonton-entry")["Image"], json!("other:1"));
}

/// 上一版由 compose 建的同名容器只带 compose 标签：认领、删掉、按现在的样子重建。
#[tokio::test]
async fn previous_compose_containers_are_adopted() {
    let rig = Rig::new().await;
    rig.docker.add_stranger_container(
        "wonton-entry",
        json!({"Image": HAPROXY_IMAGE, "Labels": {
            "com.docker.compose.project": PROJECT,
            "com.docker.compose.service": "entry",
        }}),
    );
    rig.reconcile(Some(&settings())).await.unwrap();
    let created = rig.created("wonton-entry");
    assert_eq!(created["Labels"]["wonton.core.component"], "entry");
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
}

/// 换发布端口要重建：只管“镜像没变就什么都不做”会让改了端口一直是旧的。
/// 也是换法顺序的回归测试：旧容器在跑、占着 edge 固定地址时，先建新的再删旧的会被
/// fake Docker 的地址冲突模拟挡下（复审 451 在真 Docker 上实测的失败）。
#[tokio::test]
async fn a_changed_publish_port_recreates_the_connector() {
    let rig = Rig::new().await;
    let containers = rig.containers.with_binding("127.0.0.1", "18443");
    containers.reconcile(Some(&settings())).await.unwrap();
    let containers = containers.with_binding("127.0.0.1", "18444");
    containers.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-connector"), 2);
    assert_eq!(
        rig.docker.created("wonton-connector").unwrap_or_default()["HostConfig"]["PortBindings"]["8443/tcp"],
        json!([{"HostIp": "127.0.0.1", "HostPort": "18444"}])
    );
}

/// 创建失败（端口被占、Docker 出错）时入口缺席且如实报错：换法先删旧再建新，创建失败后
/// 要等下一次对账（退避最长 10 分钟）才建回来；错误按原因报，不假装入口还在。
#[tokio::test]
async fn a_failed_recreate_reports_and_waits_for_retry() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let containers = rig.containers.with_binding("127.0.0.1", "18443");
    rig.docker
        .fail_next_create("wonton-connector", "bind: address already in use");
    let error = containers.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "conflict");
    // 旧容器已按换法删掉，新的没建成：入口缺席，等下一次对账。
    assert!(
        !rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
    // 下一次对账建回来（fail_next_create 已在失败那次被消费）。
    containers.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(
        rig.docker.created("wonton-connector").unwrap_or_default()["HostConfig"]["PortBindings"]["8443/tcp"],
        json!([{"HostIp": "127.0.0.1", "HostPort": "18443"}])
    );
}

/// fake Docker 要模拟真 Docker 的地址冲突：同一网络里运行中的容器占了固定 IP，第二个容器的 start
/// 报 Address already in use（复审 451 在真 Docker 29.7.2 上实测的原话）。
#[tokio::test]
async fn fake_docker_rejects_a_conflicting_fixed_ip_start() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_stranger_container("holder", json!({"Image": "img:1"}));
    docker.add_endpoint(
        "shared",
        "holder",
        json!({"IPAMConfig": {"IPv4Address": "10.0.0.9"}}),
    );
    docker.add_stranger_container("newcomer", json!({"Image": "img:1"}));
    docker.add_endpoint(
        "shared",
        "newcomer",
        json!({"IPAMConfig": {"IPv4Address": "10.0.0.9"}}),
    );
    let error = engine
        .start_container("newcomer")
        .await
        .expect_err("固定地址被运行中的容器占着");
    assert!(
        error.to_string().contains("Address already in use"),
        "{error}"
    );
    // 持有者停了，地址让出来，新容器起得来。
    docker.set_state(
        "holder",
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    engine.start_container("newcomer").await.unwrap();
}

/// 拉不到镜像时不动正在跑的容器，错误归 source_unavailable。
#[tokio::test]
async fn a_failed_pull_keeps_the_running_container() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.docker.fail_pulls(Some("toomanyrequests"));
    let error = rig.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "source_unavailable");
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
    assert!(
        rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
}

/// 类别不能都报成 docker_unavailable。
#[test]
fn categories_follow_the_cause() {
    use crate::plugin::docker::EngineError;
    let category = |error: EntryError| error.category();
    assert_eq!(
        category(EntryError::Docker(EngineError::PortTaken("x".into()))),
        "conflict"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::Rejected {
            context: "x".into(),
            message: "y".into()
        })),
        "invalid_input"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::NotFound("x".into()))),
        "not_found"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::Pull {
            image: "i".into(),
            message: "m".into()
        })),
        "source_unavailable"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::Unavailable("x".into()))),
        "docker_unavailable"
    );
}

fn federation_registration() -> crate::home_service::Registration {
    serde_json::from_value(json!({
        "home": "k3f9qzab",
        "domain": "k3f9qzab.example.test",
        "lan_acme_dns": {"username": "u", "password": "p", "subdomain": "s", "fulldomain": "f"},
        "acme_dns": {"username": "u", "password": "p", "subdomain": "s", "fulldomain": "f"},
        "suspended": false,
        "acme_dns_url": "https://auth.example.test/update",
        "frp_token": "token",
        "relay": {"host": "relay.example.test", "port": 443, "server_name": "relay.example.test"}
    }))
    .unwrap()
}
