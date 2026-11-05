//! ACP bridge 镜像的 Docker 集成冒烟。
//!
//! 只验证发布镜像能构建并由入口脚本启动；Matrix、Agent 和模型通路由真实开发栈冒烟验证。

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

const DOCKERFILE: &str = "deploy/dev/acp-bridge/Dockerfile";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn docker(root: &Path, args: &[&str]) -> Output {
    Command::new("docker")
        .current_dir(root)
        .args(args)
        .output()
        .expect("需要 docker 命令")
}

#[test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
fn bridge_image_starts_and_reports_missing_configuration() {
    let root = workspace_root();
    let image = format!("wonton-acp-bridge-docker-test:{}", std::process::id());
    let build_args = [
        "build", "--quiet", "--file", DOCKERFILE, "--tag", &image, ".",
    ];
    let build = docker(&root, &build_args);
    if !build.status.success() {
        let _ = docker(&root, &["rmi", "--force", &image]);
        panic!(
            "bridge 镜像构建失败：{}",
            String::from_utf8_lossy(&build.stderr)
        );
    }
    let run = docker(&root, &["run", "--rm", &image]);
    let _ = docker(&root, &["rmi", "--force", &image]);
    assert!(!run.status.success(), "缺少配置时桥不应伪装启动成功");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        output.contains("缺少环境变量"),
        "错误没有说明缺少配置：{output}"
    );
}
