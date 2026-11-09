//! 真的 gost 上的出口容器（ADR-0023）：经它到主人的代理、拒绝名单回 403、名单改了不用重建就生效、放行的地址直连、密码不进
//! `docker inspect`。
//!
//! 网络的布局和真实的一样：出口容器的默认路由在它自己的网络上，接入插件的网络和上级代理的网络。目标 `far` 只有上级代理连
//! 得到，`near` 只有出口容器直连得到，所以收到 200 就知道走了哪条路。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bollard::models::{ContainerCreateBody, HostConfig};
use wonton_core::plugin::compose::PullPolicy;
use wonton_core::plugin::core_address::Endpoint;
use wonton_core::plugin::docker::Engine;
use wonton_core::plugin::egress::address::ProxyAddress;
use wonton_core::plugin::egress::gost;
use wonton_core::plugin::egress::gost::{ALIAS, Relay, RelayFiles, config_yaml, runner};
use wonton_core::plugin::egress::lists::{self, Lists, Reach};
use wonton_core::plugin::languages::Languages;

// 共用的真实 Docker 测试辅助：每次运行唯一的实例名和资源回收。
#[path = "../src/test_docker.rs"]
mod test_docker;

/// busybox：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn busybox() -> String {
    wonton_test_images::image("public.ecr.aws/docker/library/busybox:1.37")
}
const USERNAME: &str = "u@x";
/// YAML 里有意义的字符都带上：配置不是拼出来的。
const PASSWORD: &str = "*p: #&\"w";
const ENCODED: &str = "u%40x:*p%3A%20%23%26%22w";

struct Names {
    own: String,
    plugin: String,
    upstream_network: String,
    far_network: String,
    relay: String,
    upstream: String,
    far: String,
    near: String,
    client: String,
    neighbor: String,
}

impl Names {
    /// 全部一次性资源，名字带每次运行唯一的实例名，由 `test_docker::Names` 的守卫回收。
    fn new(names: &mut test_docker::Names) -> Self {
        // 出口容器的名字和它自己的网络名是同一个。
        let relay = names.oneshot_network("gost-relay");
        Self {
            own: relay.clone(),
            plugin: names.oneshot_network("gost-plugin"),
            upstream_network: names.oneshot_network("gost-upnet"),
            far_network: names.oneshot_network("gost-farnet"),
            relay: names.register_container(relay),
            upstream: names.oneshot_container("gost-upstream"),
            far: names.oneshot_container("gost-far"),
            near: names.oneshot_container("gost-near"),
            client: names.oneshot_container("gost-client"),
            neighbor: names.oneshot_container("gost-neighbor"),
        }
    }
}

/// 回 200 的小网站，`network` 上的别名是 `alias`。
async fn website(engine: &Engine, name: &str, network: &str, alias: &str) {
    let script = "mkdir -p /www && echo ok > /www/index.html && exec httpd -f -p 80 -h /www";
    start(
        engine,
        name,
        &busybox(),
        network,
        &["sh", "-c", script],
        None,
    )
    .await;
    engine.disconnect_network(network, name).await.unwrap();
    let endpoint = Endpoint {
        address: None,
        aliases: vec![alias.to_owned()],
        gw_priority: None,
    };
    engine
        .connect_network_at(network, name, endpoint)
        .await
        .unwrap();
}

async fn start(
    engine: &Engine,
    name: &str,
    image: &str,
    network: &str,
    cmd: &[&str],
    env: Option<Vec<String>>,
) {
    let body = ContainerCreateBody {
        image: Some(image.to_owned()),
        cmd: Some(cmd.iter().map(|part| (*part).to_owned()).collect()),
        env,
        host_config: Some(HostConfig {
            network_mode: Some(network.to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(name, body).await.unwrap();
    engine.start_container(name).await.unwrap();
}

/// 客户端经代理变量访问 `url`：退出码是 0 就是 200，否则在标准错误里找状态码。
async fn fetch(engine: &Engine, client: &str, url: &str) -> Result<(), String> {
    let argv = ["wget", "-q", "-T", "20", "-O", "/dev/null", url].map(str::to_owned);
    let output = engine
        .exec(client, &argv, None, Duration::from_secs(40))
        .await
        .unwrap();
    if output.exit_code == 0 {
        Ok(())
    } else {
        Err(output.stderr)
    }
}

async fn wait_for(engine: &Engine, client: &str, url: &str, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let got = fetch(engine, client, url).await;
        let matched = match &got {
            Ok(()) => want == "200",
            Err(stderr) => stderr.contains(want),
        };
        if matched {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{url} 等不到 {want}，最后是 {got:?}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

async fn connect_relay(engine: &Engine, network: &str, relay: &str, aliases: &[&str]) {
    let endpoint = Endpoint {
        address: None,
        aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
        gw_priority: Some(-1),
    };
    engine
        .connect_network_at(network, relay, endpoint)
        .await
        .unwrap();
}

async fn start_relay(engine: &Engine, names: &Names, files: &RelayFiles, proxy: &ProxyAddress) {
    files
        .write_config(
            &config_yaml(proxy, &Languages::default()).unwrap(),
            &Languages::default(),
        )
        .unwrap();
    let labels = HashMap::new();
    let relay = Relay {
        name: &names.relay,
        labels: &labels,
        files,
        image: &gost::image(),
    };
    engine.start_relay(&relay).await.unwrap();
    connect_relay(engine, &names.plugin, &names.relay, &[ALIAS]).await;
    connect_relay(engine, &names.upstream_network, &names.relay, &[]).await;
}

/// 建好网络、上级代理和几个目标，返回上级代理的地址。
async fn set_up(engine: &Engine, names: &Names) -> (ProxyAddress, String) {
    for network in [
        &names.own,
        &names.plugin,
        &names.upstream_network,
        &names.far_network,
    ] {
        engine
            .create_relay_network(network, &HashMap::new())
            .await
            .unwrap();
    }
    // 上级代理：要用户名和密码，接着 `far` 的网络。
    let listen = format!("http://{ENCODED}@:8080");
    let upstream_cmd = ["-L", listen.as_str()];
    start(
        engine,
        &names.upstream,
        &gost::image(),
        &names.upstream_network,
        &upstream_cmd,
        None,
    )
    .await;
    let endpoint = Endpoint::default();
    engine
        .connect_network_at(&names.far_network, &names.upstream, endpoint)
        .await
        .unwrap();
    website(engine, &names.far, &names.far_network, "far.example.test").await;
    website(engine, &names.near, &names.own, "near.example.test").await;
    website(engine, &names.neighbor, &names.plugin, "neighbor.test").await;
    let upstream = engine
        .endpoint_address(&names.upstream, &names.upstream_network)
        .await
        .unwrap()
        .unwrap();
    let proxy = ProxyAddress::parse(
        &format!("http://{ENCODED}@{upstream}:8080"),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(proxy.username.as_deref(), Some(USERNAME));
    assert_eq!(proxy.password.as_deref(), Some(PASSWORD));
    (proxy, upstream)
}

/// 回环、内部名字、基础域名和别的容器的地址回 403，不到上级代理。
async fn check_denied(engine: &Engine, names: &Names) {
    let neighbor = engine
        .endpoint_address(&names.neighbor, &names.plugin)
        .await
        .unwrap()
        .unwrap();
    for url in [
        "http://127.0.0.1/".to_owned(),
        "http://localhost/".to_owned(),
        "http://localhost./".to_owned(),
        "http://x.localdomain/".to_owned(),
        "http://matrix.wonton.internal/".to_owned(),
        "http://home.example/".to_owned(),
        "http://auth.home.example/".to_owned(),
        "http://169.254.169.254/".to_owned(),
        format!("http://{neighbor}/"),
    ] {
        let got = fetch(engine, &names.client, &url).await;
        assert!(
            got.as_ref().is_err_and(|e| e.contains("403")),
            "{url}：{got:?}"
        );
    }
}

async fn exercise(engine: &Engine, names: &Names, files: &RelayFiles) {
    let (proxy, upstream) = set_up(engine, names).await;
    let denied = lists::build("home.example", &Reach::default());
    files.write_lists(&denied, &Languages::default()).unwrap();
    start_relay(engine, names, files, &proxy).await;
    let via = format!("http_proxy=http://{ALIAS}:3128");
    start(
        engine,
        &names.client,
        &busybox(),
        &names.plugin,
        &["sleep", "3600"],
        Some(vec![via]),
    )
    .await;

    // 公网（这里是只有上级代理连得到的名字）经上级代理，带着密码。
    wait_for(engine, &names.client, "http://far.example.test/", "200").await;

    check_denied(engine, names).await;

    // 放行一个地址：名单几秒内生效，容器没有重建，直接连，不经上级代理。
    let near = engine
        .endpoint_address(&names.near, &names.own)
        .await
        .unwrap()
        .unwrap();
    let url = format!("http://{near}/");
    let got = fetch(engine, &names.client, &url).await;
    assert!(got.as_ref().is_err_and(|e| e.contains("403")), "{got:?}");
    let before = relay_id(engine, &names.relay).await;
    let allowed: Lists = lists::build(
        "home.example",
        &Reach {
            direct_cidrs: vec![format!("{near}/32").parse().unwrap()],
            ..Reach::default()
        },
    );
    files.write_lists(&allowed, &Languages::default()).unwrap();
    wait_for(engine, &names.client, &url, "200").await;
    assert_eq!(relay_id(engine, &names.relay).await, before);
    wait_for(engine, &names.client, "http://far.example.test/", "200").await;

    // 密码只在配置文件里，容器的配置和日志里都没有。
    let found = engine
        .inspect_container(&names.relay)
        .await
        .unwrap()
        .unwrap();
    let text = format!("{:?}{:?}", found.config, found.args);
    assert!(!text.contains("*p:"), "{text}");
    let logs = engine.logs_tail(&names.relay, 200).await;
    assert!(
        !logs.contains("far.example.test"),
        "不记录访问了哪些网站：{logs}"
    );

    // 密码错了回 503，立刻返回。
    let wrong = ProxyAddress::parse(
        &format!("http://u:wrong@{upstream}:8080"),
        &Default::default(),
    )
    .unwrap();
    start_relay(engine, names, files, &wrong).await;
    wait_for(engine, &names.client, "http://far.example.test/", "503").await;
}

async fn relay_id(engine: &Engine, relay: &str) -> Option<String> {
    engine.inspect_container(relay).await.unwrap().unwrap().id
}

#[tokio::test]
#[ignore = "需要 Docker，起真的 gost"]
async fn the_relay_goes_through_the_owners_proxy_and_follows_its_lists() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    for image in [gost::image(), busybox()] {
        engine
            .ensure_image(&image, PullPolicy::Missing, None)
            .await
            .unwrap();
    }
    let mut names = test_docker::Names::new();
    let names = Names::new(&mut names);
    let dir = tempfile::tempdir().unwrap();
    let files = RelayFiles::new(dir.path().join("relay"), runner());
    exercise(&engine, &names, &files).await;
}
