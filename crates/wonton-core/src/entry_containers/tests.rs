//! 入口容器和连接器的期望状态：按设置建、删，安全选项逐项锁住。

use std::collections::HashMap;

use serde_json::{Value, json};

use super::{ENTRY_CLIENTS_ALIAS, ENTRY_EDGE_ALIAS, EntryContainers, EntryError};
use crate::entry_compose::HAPROXY_IMAGE;
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

struct Rig {
    docker: FakeDocker,
    containers: EntryContainers,
    dir: tempfile::TempDir,
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
        let containers = EntryContainers::new(
            engine,
            dir.path().to_owned(),
            dir.path().join("data"),
            Instance::default(),
            Some(CORE.to_owned()),
        );
        Self {
            docker,
            containers,
            dir,
        }
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
        json!(["haproxy", "-W", "-db", "-f", "/config/haproxy.cfg"])
    );
    assert_eq!(connector["Image"], HAPROXY_IMAGE);
    assert_eq!(
        connector["Cmd"],
        json!(["haproxy", "-W", "-db", "-f", "/config/connector.cfg"])
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
        assert_eq!(host["RestartPolicy"]["Name"], "unless-stopped");
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

/// 证书续期后发 `USR2` 给入口容器：按组件标签找，不看 compose 服务名。
#[tokio::test]
async fn reload_signals_the_entry_container() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload().await.unwrap();
    let kills = rig.writes_for("/kill");
    assert_eq!(kills.len(), 1, "{kills:?}");
    assert!(kills[0].contains("wonton-entry"), "{kills:?}");
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
    assert_eq!(rig.writes_for("/kill").len(), 1);
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
    let rig = Rig::new().await;
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
    rig.docker.fail_next_kill("wonton-entry", "USR2 failed");
    let error = rig
        .containers
        .apply_clients_model(&settings(), true, vec!["new".into()], vec![])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("USR2 failed"), "{error}");
    for (path, expected) in paths.iter().zip(before) {
        assert_eq!(std::fs::read(entry.join(path)).unwrap(), expected, "{path}");
    }
    assert_eq!(rig.writes_for("/kill").len(), 3);
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
