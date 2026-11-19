use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::json;
use wonton_client::{ClientCore, Network};

fn core() -> (tempfile::TempDir, std::sync::Arc<ClientCore>) {
    let directory = tempfile::tempdir().unwrap();
    let core =
        ClientCore::with_network(directory.path().to_str().unwrap(), Network::DirectOnly).unwrap();
    (directory, core)
}

/// 两端的壳按这个名字装通道：Android 的 WebMessageListener，iOS 垫到 `webkit.messageHandlers` 上。
#[test]
fn color_script_reports_over_the_page_color_channel() {
    let (_directory, core) = core();
    let script = core.page_color_script();
    assert!(script.contains("window.wontonPageColor"));
    assert!(script.contains("channel.postMessage("));
}

#[test]
#[ignore = "需要 Docker，固定摘要的官方 Node 22 验证取色脚本报的是顶上叠出来的颜色"]
fn color_script_reports_the_color_shown_at_the_top() {
    let (_directory, core) = core();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/page_color/fixture.mjs");
    let mut child = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-i",
            "--network=none",
            "--name",
            &format!("page-color-script-{}", std::process::id()),
            "--mount",
            &format!(
                "type=bind,source={},target=/fixture.mjs,readonly",
                fixture.display()
            ),
            wonton_test_images::image(
                "node:22@sha256:18a6d90fd60542269850c80ca03b29b1d9b14d6b221ac7ee8e35b0d33ef904bd",
            )
            .as_str(),
            "node",
            "/fixture.mjs",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = json!({"script": core.page_color_script()});
    writeln!(child.stdin.take().unwrap(), "{input}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["passed"], true);
}
