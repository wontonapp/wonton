//! 契约：核心向容器发信号时，那个容器必须在 `systemctl restart docker` 之后自己回来。
//!
//! Docker 把经 kill 接口发的任何信号都当成停止：容器记上手动停过的标记，`unless-stopped` 的容器重启后
//! 不再拉起（ADR-0116）。现在热加载两条路都不发信号，但机制留着：这里扫源码找 `kill_container(`，不维护
//! 容器名单，新加一处信号测试就红，要求它所在的 compose 服务或创建请求给出 `restart: always` 和
//! `stop_signal: SIGTERM`。

use std::path::{Path, PathBuf};

use serde_json::Value;

/// 核心自己渲染的两份 compose：正式安装和本地开发各一份（`install::COMPOSE` 和 dev 脚本）。
const COMPOSE: &[(&str, &str)] = &[
    (
        "deploy/install/compose.yaml",
        include_str!("../../../deploy/install/compose.yaml"),
    ),
    (
        "deploy/dev/core/compose.yaml",
        include_str!("../../../deploy/dev/core/compose.yaml"),
    ),
];

const SIGNAL: &str = "kill_container(";

/// 生产源码：不算 `*_tests.rs`、`tests.rs`、`tests/` 目录和引擎自己的 `plugin/docker/`
/// （`kill_container` 本身在那里定义）。
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|name| name != "tests" && name != "docker")
                {
                    walk(&path, found);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if !name.ends_with("_tests.rs") && name != "tests.rs" {
                    found.push(path);
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found
}

/// 发信号的文件和它里面出现的字符串字面量。
fn signal_sites() -> Vec<(PathBuf, String)> {
    sources()
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            text.contains(SIGNAL).then_some((path, text))
        })
        .collect()
}

fn quoted(text: &str) -> Vec<&str> {
    text.split('"')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, word)| word)
        .collect()
}

/// compose 里的服务名 → 服务定义。所有文件里的同名服务一起看：每个都得给保证。
fn services() -> Vec<(String, Value)> {
    COMPOSE
        .iter()
        .flat_map(|(_, text)| {
            let compose: Value = serde_saphyr::from_str(text).unwrap();
            compose["services"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, service)| (name.clone(), service.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 核心现在的两条热加载路径都不经 Docker 的信号：Tuwunel 走管理房间命令，入口走 HAProxy 主进程的管理端口；
/// 再加信号这条测试就该红。
#[test]
fn no_production_code_signals_containers() {
    let sites: Vec<_> = signal_sites()
        .into_iter()
        .map(|(path, text)| {
            let line = text
                .lines()
                .position(|line| line.contains(SIGNAL))
                .unwrap_or_default()
                + 1;
            format!("{}:{line}", path.display())
        })
        .collect();
    assert!(sites.is_empty(), "经 Docker 发信号的还有：{sites:?}");
}

/// 还在发信号的 compose 服务：所有同名服务都必须是 `restart: always`，并且停止信号是 `SIGTERM`。
///
/// 文件里的字符串字面量跟服务名对得上，就当它是发往那个服务的；对不上（容器由核心建、名字不在 compose 里）时
/// 要求同文件给出 `restart_policy: ALWAYS` 和 `stop_signal`。
#[test]
fn a_signalled_container_comes_back_after_a_docker_restart() {
    let services = services();
    for (path, text) in signal_sites() {
        let mut matched = 0;
        for word in quoted(&text) {
            for (name, service) in &services {
                if word != name {
                    continue;
                }
                matched += 1;
                assert_eq!(
                    service["restart"],
                    "always",
                    "{}：{name} 发了信号，compose 的 restart 必须是 always",
                    path.display()
                );
                assert_eq!(
                    service["stop_signal"],
                    "SIGTERM",
                    "{}：{name} 要在 Docker 重启时被 SIGTERM 停，停止信号必须是 SIGTERM",
                    path.display()
                );
            }
        }
        if matched == 0 {
            assert!(
                text.contains("RestartPolicyNameEnum::ALWAYS"),
                "{}：发信号的容器不在 compose 里，建容器时要写 restart_policy: ALWAYS",
                path.display()
            );
            assert!(
                text.contains("stop_signal: Some(\"SIGTERM\".to_owned())"),
                "{}：发信号的容器没把停止信号写成 SIGTERM",
                path.display()
            );
        }
    }
}

/// 核心自己建的容器（入口、连接器）不经过 compose：创建请求里逐项锁住重启策略。
///
/// 创建代码在 `src/entry_containers/` 的各个文件里，扫整个模块（不算测试文件）。
#[test]
fn core_created_containers_come_back_after_a_docker_restart() {
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/entry_containers");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&module)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_some_and(|name| name != "tests.rs")
        })
        .collect();
    files.sort();
    let text: String = files
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    for policy in text.match_indices("RestartPolicyNameEnum::") {
        assert!(
            text[policy.0..].starts_with("RestartPolicyNameEnum::ALWAYS"),
            "入口容器的 restart_policy 只许是 ALWAYS"
        );
    }
    assert!(
        text.contains("stop_signal: Some(\"SIGTERM\".to_owned())"),
        "入口容器没有把停止信号写成 SIGTERM"
    );
}
