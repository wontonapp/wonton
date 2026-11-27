//! 入口运行循环：三种装法下续期都要走到、入口容器都不能被对账删掉。
//!
//! 时钟用 tokio 的假时钟推 90 天，不 sleep；签发在假 Docker 上必然失败（lego 没真的跑），
//! 所以“续期发生了”看的是签发有没有起 lego 容器（`recorded_creations`），不是签发成功与否。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
use wonton_protocol::{CredentialEntry, PutDomainRequest, SecretString};

use super::spawn;
use crate::clients::domain::Domain;
use crate::entry_containers::EntryContainers;
use crate::entry_runtime::FRPC_IMAGE;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::names::Instance;

const CORE: &str = "wonton-core";
const PROJECT: &str = "wonton";

struct Rig {
    docker: FakeDocker,
    engine: Engine,
    domain: Arc<Domain>,
    dir: tempfile::TempDir,
}

impl Rig {
    /// 一台装了核心的机器：compose 网络和核心容器在，入口证书已经签出来过。
    async fn new() -> Self {
        unsafe { std::env::set_var("WONTON_ACME_DIRECTORY", "https://pebble.test:14000/dir") };
        let (docker, engine) = FakeDocker::start().await;
        docker.add_stranger_container(
            CORE,
            json!({"Labels": {"com.docker.compose.project": PROJECT}}),
        );
        for network in ["clients", "edge", "relay"] {
            docker.add_unlabeled_network(&format!("{PROJECT}_{network}"));
        }
        let dir = tempfile::tempdir().unwrap();
        let tls = rcgen::generate_simple_self_signed(vec!["home.example.test".to_owned()]).unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), tls.cert.pem()).unwrap();
        std::fs::write(
            dir.path().join("entry/certs/site.key"),
            tls.signing_key.serialize_pem(),
        )
        .unwrap();
        let domain = Domain::new(
            dir.path().to_owned(),
            dir.path().join("data"),
            Engine::for_test(),
            None,
        );
        Self {
            docker,
            engine,
            domain,
            dir,
        }
    }

    fn containers(&self) -> EntryContainers {
        EntryContainers::new(
            self.engine.clone(),
            self.dir.path().to_owned(),
            self.dir.path().join("data"),
            Instance::default(),
            Some(CORE.to_owned()),
        )
        .with_binding("127.0.0.1", "18443")
    }

    fn access(&self) -> Arc<crate::entry_access::EntryAccess> {
        let access = Arc::new(
            crate::entry_access::EntryAccess::open(self.dir.path().join("core/entry-access.json"))
                .unwrap(),
        );
        access
            .save(wonton_protocol::PublicAccess { enabled: true })
            .unwrap();
        access
    }

    fn entries(&self) -> usize {
        self.docker
            .containers()
            .iter()
            .filter(|name| name.as_str() == "wonton-entry")
            .count()
    }

    fn created(&self) -> bool {
        let names = self.docker.containers();
        names.contains(&"wonton-entry".to_owned()) && names.contains(&"wonton-connector".to_owned())
    }

    fn attempts(&self) -> usize {
        self.domain.engine.recorded_creations().len()
    }
}

fn settings(dir: &std::path::Path, value: serde_json::Value) {
    let path = dir.join("entry/settings.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

/// 装机时用了自己的域名：设置文件写好了名字、服务商、凭据和条款。
fn own_domain_settings(dir: &std::path::Path) {
    let env_file = dir.join("provider.env");
    std::fs::write(&env_file, "HTTPREQ_ENDPOINT=https://relay.test/present\n").unwrap();
    settings(
        dir,
        json!({
            "domain": "home.example.test",
            "service_url": null,
            "service_ca": null,
            "relay_ca": null,
            "registration": null,
            "acme_directory": "https://pebble.test:14000/dir",
            "acme_ca": null,
            "acme_resolvers": [],
            "acme_email": null,
            "acme_accept_terms": true,
            "acme_network": "bridge",
            "dns_provider": "httpreq",
            "dns_env_file": env_file,
            "external_certificate": false
        }),
    );
}

/// 送名字的家名服务：注册信息和中继都在设置里，条款装机时给了。
fn registered_settings(dir: &std::path::Path) {
    let credentials = json!({
        "username": "test", "password": "test", "subdomain": "test",
        "fulldomain": "test.auth.example.test"
    });
    // 证书同时覆盖 `*.lan`，局域网那个名字要另一套 acme-dns 账号。
    let lan = json!({
        "username": "test", "password": "test", "subdomain": "lan",
        "fulldomain": "lan.auth.example.test"
    });
    let ca = dir.join("relay-ca.pem");
    std::fs::write(&ca, "ca").unwrap();
    settings(
        dir,
        json!({
            "domain": "home.example.test",
            "service_url": null,
            "service_ca": null,
            "relay_ca": ca,
            "registration": {
                "home": "home", "domain": "home.example.test", "frp_token": "test",
                "acme_dns": credentials, "lan_acme_dns": lan,
                "acme_dns_url": "https://auth.example.test",
                "relay": {"host": "relay.example.test", "port": 443, "server_name": "relay.example.test"},
                "suspended": false
            },
            "acme_directory": "https://pebble.test:14000/dir",
            "acme_ca": null,
            "acme_resolvers": [],
            "acme_email": null,
            "acme_accept_terms": true,
            "acme_network": "bridge",
            "dns_provider": null,
            "dns_env_file": null,
            "external_certificate": false
        }),
    );
}

fn request(suffix: &str) -> PutDomainRequest {
    PutDomainRequest {
        suffix: Some(suffix.to_owned()),
        provider: "httpreq".into(),
        credentials: vec![CredentialEntry {
            name: "HTTPREQ_ENDPOINT".into(),
            value: SecretString::new("https://relay.test/present"),
        }],
        accept_terms: true,
    }
}

/// 推一段假时间，让到点的轮次都跑完。
async fn tick(by: Duration) {
    tokio::time::advance(by).await;
    for _ in 0..2_000 {
        tokio::task::yield_now().await;
    }
}

/// 一秒一秒推时间直到成立；推 6 小时还不成立就算失败。
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..21_600 {
        if ready() {
            return;
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
    }
    panic!("等不到：{what}");
}

/// 等排队的签发都收尾，后续计数才不会被它们的余波干扰。
async fn settle(domain: &Domain) {
    for _ in 0..20_000 {
        if domain.pending.load(Ordering::SeqCst) == 0 {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("签发没在限定的轮数里结束");
}

/// 先“不要名字”装好，再在 App 里填自己的域名：入口要建起来、留着，之后每小时检查要走到续期。
#[tokio::test(start_paused = true)]
async fn none_mode_renews_after_the_owner_sets_a_domain() {
    let rig = Rig::new().await;
    spawn(
        Arc::new(rig.containers()),
        rig.access(),
        rig.domain.clone(),
        None,
        None,
    );
    // 没配域名：没有要服务的东西，也不该联系 CA。
    tick(Duration::from_secs(3 * 3600)).await;
    assert_eq!(rig.entries(), 0);
    assert_eq!(rig.attempts(), 0);
    // 主人在 App 里勾了条款、填了域名。
    rig.domain.save(request("home.example.test")).await.unwrap();
    settle(&rig.domain).await;
    let saved = rig.attempts();
    // 下一轮对账把入口建起来，不按“没有设置文件”删掉。
    until("入口和连接器建起来", || rig.created()).await;
    // 每小时的检查走到续期：一天后要再签一次。
    tick(Duration::from_secs(25 * 3600)).await;
    until("一天后有续期", || rig.attempts() > saved).await;
    // 90 天后证书早该续过一大串，入口容器一直没被删。
    let before = rig.attempts();
    tick(Duration::from_secs(90 * 24 * 3600)).await;
    until("90 天里天天续期", || rig.attempts() >= before + 89).await;
    assert_eq!(rig.entries(), 1);
}

/// 装机时用自己的域名：入口建起来，之后每小时检查要走到续期。
#[tokio::test(start_paused = true)]
async fn own_domain_install_renews() {
    let rig = Rig::new().await;
    own_domain_settings(rig.dir.path());
    spawn(
        Arc::new(rig.containers()),
        rig.access(),
        rig.domain.clone(),
        None,
        None,
    );
    until("入口和连接器建起来", || rig.created()).await;
    until("起来那一轮就排一次续期", || rig.attempts() > 0).await;
    let before = rig.attempts();
    tick(Duration::from_secs(90 * 24 * 3600)).await;
    until("90 天里天天续期", || rig.attempts() >= before + 89).await;
    assert_eq!(rig.entries(), 1);
}

/// 送名字（家名服务）：连接器是 frpc，续期照样走。
#[tokio::test(start_paused = true)]
async fn given_name_install_renews() {
    let rig = Rig::new().await;
    registered_settings(rig.dir.path());
    spawn(
        Arc::new(rig.containers()),
        rig.access(),
        rig.domain.clone(),
        None,
        None,
    );
    until("入口和连接器建起来", || rig.created()).await;
    let connector = rig
        .docker
        .created("wonton-connector")
        .expect("连接器建起来了");
    assert_eq!(connector["Image"], FRPC_IMAGE);
    until("起来那一轮就排一次续期", || rig.attempts() > 0).await;
    let before = rig.attempts();
    tick(Duration::from_secs(90 * 24 * 3600)).await;
    until("90 天里天天续期", || rig.attempts() >= before + 89).await;
    assert_eq!(rig.entries(), 1);
}

/// 新证书发布后运行循环收到唤醒，不等 30 秒的退避就重对一次账。
#[tokio::test(start_paused = true)]
async fn a_published_certificate_wakes_the_loop() {
    let rig = Rig::new().await;
    std::fs::write(
        rig.dir.path().join("entry/domain.json"),
        json!({"suffix": "home.example.test"}).to_string(),
    )
    .unwrap();
    spawn(
        Arc::new(rig.containers()),
        rig.access(),
        rig.domain.clone(),
        None,
        None,
    );
    // 第一轮对账在 30 秒后退避到点，测试不等钟；唤醒要立刻建起入口。
    assert!(!rig.created());
    rig.domain.notify_refresh();
    until("被唤醒的那轮把入口建起来", || rig.created()).await;
    assert!(rig.dir.path().join("entry/haproxy.cfg").is_file());
}
