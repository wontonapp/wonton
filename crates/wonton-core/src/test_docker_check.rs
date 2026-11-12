//! 真实 Docker 测试的文件检查：不许出现写死的 `wonton-` 资源名或默认实例（正式安装的名字），
//! 用到的镜像引用必须在 `scripts/mirror-test-images.sh` 的清单里。CI 的几个 runner 共用一台 Docker，
//! 写死的名字会被并行的作业或清理脚本撞掉、删掉（#569）；清单外的镜像预拉会跳过，测试才 `No such image`。

use std::path::{Path, PathBuf};
use std::process::Command;

/// Docker 测试里出现的镜像引用必须都在镜像清单里：预拉只认 `scripts/mirror-test-images.sh` 的清单，
/// 引用绕开 `wonton_test_images::image` 写死（或把原始引用直接传给 `ensure_image`）时预拉会跳过它，
/// 测试跑到一半才 `No such image`。这是写法错误，不是拉取抖动，直接失败。
#[test]
fn docker_test_images_are_in_the_mirror_list() {
    const PREFIX: &str = "ghcr.io/example/test-images";
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = root.join("../..").canonicalize().unwrap();
    let mut sources = Vec::new();
    collect(&workspace.join("crates"), &workspace, &mut sources);

    let listed = Command::new("bash")
        .arg(workspace.join("scripts/mirror-test-images.sh"))
        .arg("--list")
        .env("WONTON_TEST_IMAGES_PREFIX", PREFIX)
        .output()
        .expect("scripts/mirror-test-images.sh 要能跑");
    assert!(
        listed.status.success(),
        "scripts/mirror-test-images.sh --list 失败：{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed = String::from_utf8_lossy(&listed.stdout).into_owned();

    // SAFETY: 进程级环境变量。nextest 每个测试一个进程；同进程里并行的测试只是读，映射结果和前缀无关。
    let previous = std::env::var_os("WONTON_TEST_IMAGES");
    unsafe { std::env::set_var("WONTON_TEST_IMAGES", PREFIX) };
    let mut missing = Vec::new();
    for (path, content) in &sources {
        if !content.contains("#[ignore = \"需要 Docker") {
            continue;
        }
        for (line, number) in content.lines().zip(1..) {
            for literal in quoted_literals(line).filter(|literal| image_like(literal)) {
                let mapped = wonton_test_images::image(literal);
                if !listed.lines().any(|known| known == mapped) {
                    missing.push(format!("{path}:{number}: {literal}"));
                }
            }
        }
    }
    match previous {
        Some(value) => unsafe { std::env::set_var("WONTON_TEST_IMAGES", value) },
        None => unsafe { std::env::remove_var("WONTON_TEST_IMAGES") },
    }

    assert!(
        missing.is_empty(),
        "这些镜像没进 scripts/mirror-test-images.sh，预拉会跳过（写法错误，不是基础设施失败）：\n{}\n\
         镜像要经 wonton_test_images::image 取（引用放在 const 里也行），并把来源补进 mirror 脚本。",
        missing.join("\n")
    );
}

/// 一行里被双引号包起来的字面量。`\"` 等转义不处理：镜像引用里没有转义。
fn quoted_literals(line: &str) -> impl Iterator<Item = &str> {
    line.split('"').skip(1).step_by(2)
}

/// 像镜像引用的字面量：带仓库路径，并且带摘要或标签。路径、URL、格式串靠这几条排除。
fn image_like(literal: &str) -> bool {
    if literal.starts_with("http")
        || literal.contains(['{', '$', '\\'])
        || literal.contains(char::is_whitespace)
    {
        return false;
    }
    if let Some((repo, digest)) = literal.split_once('@') {
        return repo.contains('/') && sha256(digest);
    }
    let Some((repo, tag)) = literal.rsplit_once(':') else {
        return false;
    };
    repo.contains('/') && !tag.is_empty() && !tag.contains('/')
}

fn sha256(digest: &str) -> bool {
    digest
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[test]
fn docker_test_files_use_unique_resource_names() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    collect(&root.join("src"), &root, &mut sources);
    collect(&root.join("tests"), &root, &mut sources);
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

/// 收集目录下的 `.rs` 文件（相对 `base` 的路径，配上内容）。
fn collect(dir: &Path, base: &Path, into: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.map_while(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, base, into);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let relative = path.strip_prefix(base).unwrap_or(&path).display();
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
