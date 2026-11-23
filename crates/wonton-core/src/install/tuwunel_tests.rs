//! 全新安装的 Tuwunel 起得来：`init` 写出的配置，加上正式 compose 里给 tuwunel 的命令、环境变量和挂载，起真的 Tuwunel。
//! 配置里引用的文件没挂进容器时它当场退出（#605 漏挂 TLS 证书，全新安装就卡在这一步）。

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::test_docker::Names;

const WAIT: Duration = Duration::from_secs(90);

/// 只读 tuwunel：别的服务里有长格式的挂载（核心的 /dev），按这里的类型读不了。
#[derive(Deserialize)]
struct Compose {
    services: Services,
}

#[derive(Deserialize)]
struct Services {
    tuwunel: Service,
}

#[derive(Deserialize)]
struct Service {
    image: String,
    #[serde(default)]
    command: Vec<String>,
    /// `<<` 合并键读出来可能是映射，只带值是字符串的。
    #[serde(default)]
    environment: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    volumes: Vec<String>,
    restart: Option<String>,
    stop_signal: Option<String>,
}

/// Docker 重启、容器崩溃都要自己回来：compose 写死 `restart: always` 和停止信号 `SIGTERM`。
/// `unless-stopped` 会被历史上任何一次非停止 kill 记成手动停过，守护进程重启后不再拉起；没写停止信号时信号还会停掉
/// 重启管理器，之后容器崩了也不拉起。
#[test]
fn tuwunel_comes_back_after_docker_restarts_even_when_signalled() {
    let compose: Compose = serde_saphyr::from_str(super::COMPOSE).unwrap();
    let tuwunel = compose.services.tuwunel;
    assert_eq!(tuwunel.restart.as_deref(), Some("always"));
    assert_eq!(tuwunel.stop_signal.as_deref(), Some("SIGTERM"));
}

#[test]
#[ignore = "需要 Docker，跑一遍约半分钟"]
fn a_fresh_install_starts_tuwunel_with_the_compose_mounts() {
    let dir = tempfile::tempdir().unwrap();
    super::init(dir.path(), None).unwrap();
    let mut names = Names::new();
    let name = start(dir.path(), &mut names);
    wait_healthy(&name);
}

/// 有名字、没配入口的安装：Tuwunel 起得来，联邦端口仍在本机听着（核心的投放地址指着它），但没有 TLS，
/// 对端（一律拨 `https`）会停在握手，配好入口才通（ADR-0090）。
#[test]
#[ignore = "需要 Docker，跑一遍约半分钟"]
fn a_named_install_without_an_entry_listens_plain_on_the_federation_port() {
    let dir = tempfile::tempdir().unwrap();
    super::init(dir.path(), Some("home.example")).unwrap();
    let mut names = Names::new();
    let name = start(dir.path(), &mut names);
    wait_healthy(&name);
    let ip = container_ip(&name);

    let body = String::from_utf8(get(&ip, 8008, "/_matrix/client/versions")).unwrap();
    assert!(body.contains("versions"), "{body}");
    // 核心把 iroh 进来的流投到这个端口：它得有监听，而且回得出应答。
    let fed = String::from_utf8(get(&ip, 8448, "/_matrix/client/versions")).unwrap();
    assert!(fed.contains("versions"), "{fed}");
}

/// 有入口设置的安装：TLS 全交给入口容器，Tuwunel 不再听 8448（公网 8448 不发布）。
#[test]
#[ignore = "需要 Docker，跑一遍约半分钟"]
fn a_named_install_with_an_entry_does_not_listen_on_8448() {
    let dir = tempfile::tempdir().unwrap();
    super::entry_mode_tests::named_settings(dir.path());
    super::init(dir.path(), Some("home.example")).unwrap();
    let mut names = Names::new();
    let name = start(dir.path(), &mut names);
    wait_healthy(&name);
    let ip = container_ip(&name);

    assert!(
        String::from_utf8(get(&ip, 8008, "/_matrix/client/versions"))
            .unwrap()
            .contains("versions")
    );
    assert!(
        std::net::TcpStream::connect((ip.as_str(), 8448)).is_err(),
        "8448 不该有监听"
    );
}

/// 按生产 compose 的挂载、环境变量和命令起一台 Tuwunel；返回容器名（由 `names` 负责删）。
fn start(install_dir: &Path, names: &mut Names) -> String {
    let compose: Compose = serde_saphyr::from_str(super::COMPOSE).unwrap();
    let tuwunel = compose.services.tuwunel;
    let name = names.oneshot_container("tuwunel");
    let mut args = vec![
        "run".to_owned(),
        "--detach".to_owned(),
        "--name".to_owned(),
        name.clone(),
    ];
    args.extend(bind_mounts(&tuwunel.volumes, install_dir));
    // 本机 Docker 客户端配了代理时会注入容器；compose 里也是全部置空。
    for var in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
        args.extend(["--env".to_owned(), format!("{var}=")]);
    }
    // 健康检查（`tuwunel --health-check`）不带 -c，靠 TUWUNEL_CONFIG 找配置。
    for (var, value) in &tuwunel.environment {
        if let Some(value) = value.as_str() {
            args.extend(["--env".to_owned(), format!("{var}={value}")]);
        }
    }
    // 镜像引用经 WONTON_TEST_IMAGES 指到 CI 的副本（wonton-test-images）；docker run 按摘要取，
    // 本地和 CI 用的是同一个引用。
    args.push(wonton_test_images::image(&tuwunel.image));
    args.extend(tuwunel.command);
    let started = docker(&args);
    assert!(
        started.status.success(),
        "起 Tuwunel：{}",
        String::from_utf8_lossy(&started.stderr)
    );
    name
}

fn wait_healthy(name: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        if docker(&["exec", name, "tuwunel", "--health-check"])
            .status
            .success()
        {
            return;
        }
        let running = docker(&["inspect", "--format", "{{.State.Running}}", name]);
        let gone = String::from_utf8_lossy(&running.stdout).trim() != "true";
        if gone || Instant::now() > deadline {
            let logs = docker(&["logs", "--tail", "30", name]);
            panic!(
                "Tuwunel 没有起来（{}）：\n{}{}",
                if gone { "已退出" } else { "超时" },
                String::from_utf8_lossy(&logs.stdout),
                String::from_utf8_lossy(&logs.stderr)
            );
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn container_ip(name: &str) -> String {
    let inspected = docker(&[
        "inspect",
        "--format",
        "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
        name,
    ]);
    let ip = String::from_utf8_lossy(&inspected.stdout).trim().to_owned();
    assert!(!ip.is_empty(), "容器没有地址");
    ip
}

/// 明文 GET，返回响应体；容器刚起来时可能还没开始收连接，连不上就重试到超时。
fn get(ip: &str, port: u16, path: &str) -> Vec<u8> {
    use std::io::{Read, Write};

    let deadline = Instant::now() + WAIT;
    let mut last = String::new();
    while Instant::now() < deadline {
        match std::net::TcpStream::connect((ip, port)) {
            Ok(mut tcp) => {
                let request =
                    format!("GET {path} HTTP/1.1\r\nHost: {ip}\r\nConnection: close\r\n\r\n");
                tcp.write_all(request.as_bytes()).unwrap();
                let mut raw = Vec::new();
                tcp.read_to_end(&mut raw).unwrap();
                let text = String::from_utf8_lossy(&raw).into_owned();
                let (head, body) = text.split_once("\r\n\r\n").expect("有响应头");
                assert!(head.starts_with("HTTP/1.1 200"), "{head}");
                return body.as_bytes().to_vec();
            }
            Err(error) => last = error.to_string(),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    panic!("连不上 {ip}:{port}{path}：{last}");
}

/// compose 里的绑定挂载换成安装目录下的真实路径；命名卷（数据库）不挂，容器里用临时的。
fn bind_mounts(volumes: &[String], install_dir: &Path) -> Vec<String> {
    let install_dir = install_dir.to_string_lossy();
    volumes
        .iter()
        .filter(|volume| volume.starts_with("${WONTON_INSTALL_DIR}/"))
        .flat_map(|volume| {
            let mount = volume.replace("${WONTON_INSTALL_DIR}", &install_dir);
            ["--volume".to_owned(), mount]
        })
        .collect()
}

fn docker<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Output {
    Command::new("docker")
        .args(args)
        .output()
        .expect("运行 Docker")
}
