//! 附件：图片按 `promptCapabilities` 分流 image/resource_link，文件落工作区给
//! `resource_link`；下载失败、超限按类别报错且不调 Agent；语音照旧不支持。

use base64::Engine as _;
use serde_json::Value;
use wonton_matrix::{FileContent, ImageContent, MessageBody};

use super::*;
use crate::bridge::attachments::MAX_ATTACHMENT_BYTES;
use crate::bridge::test_support::owner_message_with;

/// 造一张主人发的图片消息，把字节登记进假房间，返回（消息，字节）。
fn image_message(harness: &Harness, event_id: &str, bytes: Vec<u8>, caption: Option<&str>) {
    let image = ImageContent::for_test(
        "照片.jpg",
        Some("image/jpeg"),
        Some(bytes.len() as u64),
        caption,
        "mxc://example.org/photo",
    );
    harness
        .room
        .serve_media(&image.media_token(), bytes.clone());
    harness.room.push(vec![owner_message_with(
        event_id,
        MessageBody::Image(image),
    )]);
}

/// 等到 Agent 手上出现一轮，返回它的内容块。
async fn wait_blocks(harness: &Harness) -> Value {
    let agent = &harness.agent;
    wait_until(|| agent.pending_count() > 0).await;
    harness.agent.prompt_blocks().await.remove(0).1
}

#[tokio::test]
async fn an_image_goes_inline_for_an_agent_that_declares_image() {
    let workspace = tempfile::tempdir().unwrap();
    let harness = Harness::in_scenario(Scenario::ManualImages, |config| {
        config.workspace = workspace.path().display().to_string();
    })
    .await;
    let bytes = vec![7_u8; 32];
    image_message(
        &harness,
        "$img:example.org",
        bytes.clone(),
        Some("这是什么花"),
    );
    let blocks = wait_blocks(&harness).await;
    assert_eq!(blocks.as_array().unwrap().len(), 2, "说明和图片同一轮");
    assert_eq!(blocks[0]["type"], "text");
    assert!(blocks[0]["text"].as_str().unwrap().contains("这是什么花"));
    assert_eq!(blocks[1]["type"], "image");
    assert_eq!(blocks[1]["mimeType"], "image/jpeg");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(blocks[1]["data"].as_str().unwrap())
            .unwrap(),
        bytes
    );
    // 图片不需要留在工作区也能交付，但已经落了盘就归清理策略管。
    harness.finish().await;
}

#[tokio::test]
async fn an_image_becomes_a_resource_link_without_the_image_capability() {
    let workspace = tempfile::tempdir().unwrap();
    let harness = Harness::with(|config| {
        config.workspace = workspace.path().display().to_string();
    })
    .await;
    let bytes = vec![9_u8; 16];
    image_message(&harness, "$img:example.org", bytes.clone(), None);
    let blocks = wait_blocks(&harness).await;
    assert_eq!(blocks.as_array().unwrap().len(), 1, "没有说明就只有附件块");
    assert_eq!(blocks[0]["type"], "resource_link");
    assert_eq!(blocks[0]["name"], "照片.jpg");
    assert_eq!(blocks[0]["mimeType"], "image/jpeg");
    assert_eq!(blocks[0]["size"], 16);
    let uri = blocks[0]["uri"].as_str().unwrap();
    let path = url::Url::parse(uri)
        .unwrap()
        .to_file_path()
        .expect("file:// 路径");
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    // 按房间/事件分目录，路径在工作区的附件目录里。
    assert!(path.starts_with(workspace.path().join("wonton-attachments")));
    harness.finish().await;
}

#[tokio::test]
async fn a_file_lands_in_the_workspace_with_a_clean_name() {
    let workspace = tempfile::tempdir().unwrap();
    let harness = Harness::with(|config| {
        config.workspace = workspace.path().display().to_string();
    })
    .await;
    let bytes = b"pdf-bytes".to_vec();
    let file = FileContent::for_test(
        "../../etc/报表.pdf",
        Some("application/pdf"),
        Some(bytes.len() as u64),
        None,
        "mxc://example.org/report",
    );
    harness.room.serve_media(&file.media_token(), bytes.clone());
    harness.room.push(vec![owner_message_with(
        "$file:example.org",
        MessageBody::File(file),
    )]);
    let blocks = wait_blocks(&harness).await;
    assert_eq!(blocks[0]["type"], "resource_link");
    assert_eq!(blocks[0]["name"], "报表.pdf", "只留文件名，不带路径");
    let path = url::Url::parse(blocks[0]["uri"].as_str().unwrap())
        .unwrap()
        .to_file_path()
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    assert!(path.starts_with(workspace.path().join("wonton-attachments")));
    harness.finish().await;
}

#[tokio::test]
async fn a_failed_download_reports_by_category_and_skips_the_agent() {
    let harness = Harness::start().await;
    // 不往假房间登记句柄：下载必失败。
    let image = ImageContent::for_test(
        "照片.jpg",
        Some("image/jpeg"),
        Some(4),
        None,
        "mxc://example.org/missing",
    );
    harness.room.push(vec![owner_message_with(
        "$bad:example.org",
        MessageBody::Image(image),
    )]);
    wait_until(|| !harness.room.main_texts().is_empty()).await;
    let reply = harness.room.main_texts().remove(0);
    assert!(
        reply.starts_with("Matrix 错误："),
        "要按类别报错，收到：{reply}"
    );
    assert!(harness.prompts().await.is_empty(), "Agent 不该被调");
    harness.finish().await;
}

#[tokio::test]
async fn an_oversized_attachment_is_rejected_before_download() {
    let workspace = tempfile::tempdir().unwrap();
    let harness = Harness::with(|config| {
        config.workspace = workspace.path().display().to_string();
    })
    .await;
    let image = ImageContent::for_test(
        "大图.jpg",
        Some("image/jpeg"),
        Some(MAX_ATTACHMENT_BYTES as u64 + 1),
        None,
        "mxc://example.org/big",
    );
    harness.room.push(vec![owner_message_with(
        "$big:example.org",
        MessageBody::Image(image),
    )]);
    wait_until(|| !harness.room.main_texts().is_empty()).await;
    let reply = harness.room.main_texts().remove(0);
    assert_eq!(reply, "输入非法：附件超过 20 MB 的上限");
    assert!(harness.prompts().await.is_empty());
    harness.finish().await;
}

#[tokio::test]
async fn an_attachment_that_lies_about_its_size_is_caught_after_download() {
    let harness = Harness::start().await;
    // 事件声明很小，实际字节超限：下载后按实际字节数复核。
    let image = ImageContent::for_test(
        "大图.jpg",
        Some("image/jpeg"),
        Some(4),
        None,
        "mxc://example.org/liar",
    );
    harness
        .room
        .serve_media(&image.media_token(), vec![0; MAX_ATTACHMENT_BYTES + 1]);
    harness.room.push(vec![owner_message_with(
        "$liar:example.org",
        MessageBody::Image(image),
    )]);
    wait_until(|| !harness.room.main_texts().is_empty()).await;
    assert!(harness.room.main_texts().remove(0).contains("附件超过"));
    assert!(harness.prompts().await.is_empty());
    harness.finish().await;
}

#[tokio::test]
async fn audio_still_reports_unsupported_by_kind() {
    let harness = Harness::start().await;
    harness.room.push(vec![owner_message_with(
        "$audio:example.org",
        MessageBody::Other {
            kind: "m.audio".to_owned(),
        },
    )]);
    wait_until(|| !harness.room.main_texts().is_empty()).await;
    let reply = harness.room.main_texts().remove(0);
    assert!(reply.contains("语音消息"), "要写清是哪种：{reply}");
    assert!(reply.contains("不支持"));
    assert!(harness.prompts().await.is_empty());
    harness.finish().await;
}
