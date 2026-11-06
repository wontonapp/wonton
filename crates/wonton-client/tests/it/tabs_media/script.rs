use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::json;
use wonton_client::PageMediaAction;

const NODE_IMAGE: &str =
    "node:22@sha256:18a6d90fd60542269850c80ca03b29b1d9b14d6b221ac7ee8e35b0d33ef904bd";

#[test]
#[ignore = "需要 Docker，固定摘要的官方 Node 22 验证网页媒体状态和系统命令往返"]
fn media_script_roundtrip() {
    let (_directory, core) = super::core();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/tabs_media/fixture.mjs");
    let mut child = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-i",
            "--network=none",
            "--name",
            &format!("page-media-script-{}", std::process::id()),
            "--mount",
            &format!(
                "type=bind,source={},target=/fixture.mjs,readonly",
                fixture.display()
            ),
            NODE_IMAGE,
            "node",
            "/fixture.mjs",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = json!({"script":core.page_media_script(),"commands":[
        core.page_media_command_script(PageMediaAction::NextTrack, None),
        core.page_media_command_script(PageMediaAction::SeekTo, Some(24500)),
        core.page_media_command_script(PageMediaAction::Pause, None)
    ]});
    writeln!(child.stdin.take().unwrap(), "{input}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["passed"], true);
}
