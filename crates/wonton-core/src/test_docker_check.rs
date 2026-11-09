//! 真实 Docker 测试的命名检查：这些文件里不许出现写死的 `wonton-` 资源名或默认实例（正式安装的名字），
//! CI 的几个 runner 共用一台 Docker，写死的名字会被并行的作业或清理脚本撞掉、删掉（#569）。

use std::path::{Path, PathBuf};

#[test]
fn docker_test_files_use_unique_resource_names() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    collect(&root.join("src"), &mut sources);
    collect(&root.join("tests"), &mut sources);
    let hardcoded = sources
        .into_iter()
        .filter(|(_, content)| content.contains("#[ignore = \"需要 Docker"))
        .flat_map(|(path, content)| {
            content
                .lines()
                .zip(1..)
                .filter_map(move |(line, number)| {
                    hardcoded(line).map(|why| format!("{path}:{number}: {why}"))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(
        hardcoded.is_empty(),
        "真实 Docker 测试里写死了资源名或默认实例：\n{}\n资源名要经 test_docker::Names（每次运行唯一的 \
         实例名）拼出来，见 crates/wonton-core/src/test_docker.rs",
        hardcoded.join("\n")
    );
}

/// 收集目录下的 `.rs` 文件（相对 crate 根的路径，配上内容）。
fn collect(dir: &Path, into: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.map_while(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, into);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let relative = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap()
                .display();
            into.push((
                relative.to_string(),
                std::fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

/// 一行里的写死名字。`wonton-core` 是假核心镜像里的可执行文件名（core_upgrade 的 compose 和
/// ENTRYPOINT，含转义形式），不是 Docker 资源名。
fn hardcoded(line: &str) -> Option<&'static str> {
    if line.contains("\"wonton-core") {
        return None;
    }
    if line.contains("\"wonton-") {
        return Some("写死了 wonton- 开头的资源名");
    }
    for symbol in ["manager_with", "manager_in_container", "Instance::default"] {
        if line.contains(symbol) {
            return Some(
                "用了默认实例：真实 Docker 的测试经 test_docker::Names 的唯一实例名建管理器",
            );
        }
    }
    None
}
