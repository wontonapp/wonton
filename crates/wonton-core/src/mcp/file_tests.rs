//! 工具结果里的文件和 Bot 的文件工具（ADR-0020 的“Bot”）：假插件返回引用，断言 Agent 看到什么、主人后来能不能下载。

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolResult, ContentBlock, MetaObject, Resource};
use serde_json::{Value, json};
use wonton_protocol::{FileFailure, FileId, FileInfo, FileSource};

use super::tests::{Setup, failed, setup, text};
use crate::files::record::Accessor;
use crate::files::test_support::Fixture;
use crate::plugin::docker::Engine as Docker;

const AGENT: &str = "com.example.agent";
const NOTES: &str = "app.example.notes";
const FIND: &str = "app-example-notes__find";

struct Bench {
    s: Setup,
    f: Fixture,
}

async fn bench() -> Bench {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    let f = Fixture::new();
    s.bus.attach_files(f.files.clone());
    Bench { s, f }
}

fn jpeg(edge: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(edge, edge)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

fn share_link(path: &str) -> ContentBlock {
    let name = path.rsplit('/').next().unwrap();
    ContentBlock::ResourceLink(Resource::new(format!("wonton-share:{path}"), name))
}

fn images(result: &CallToolResult) -> Vec<(u32, u32)> {
    result
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Image(image) => {
                assert_eq!(image.mime_type, "image/jpeg");
                let bytes = STANDARD.decode(&image.data).unwrap();
                let decoded = image::load_from_memory(&bytes).unwrap();
                Some((decoded.width(), decoded.height()))
            }
            _ => None,
        })
        .collect()
}

/// 结果文字里写的编号。
fn ids(result: &CallToolResult) -> Vec<FileId> {
    text(result)
        .split("编号 ")
        .skip(1)
        .map(|rest| rest[..26].parse().unwrap())
        .collect()
}

impl Bench {
    async fn find(&self, blocks: Vec<ContentBlock>) -> CallToolResult {
        *self.s.notes.found.lock().unwrap() = CallToolResult::success(blocks);
        self.s.call(FIND, json!({})).await
    }

    async fn found(&self, path: &str, content: &[u8]) -> FileId {
        self.f.put(path, content);
        let result = self.find(vec![share_link(path)]).await;
        ids(&result).pop().expect("应该登记成编号")
    }

    async fn view(&self, id: &FileId) -> CallToolResult {
        self.s
            .call("wonton__view_file", json!({"file_id": id.as_str()}))
            .await
    }

    async fn share(&self, id: &FileId) -> CallToolResult {
        self.s
            .call("wonton__share_file", json!({"file_id": id.as_str()}))
            .await
    }

    async fn owner_info(&self, id: &FileId) -> Result<FileInfo, FileFailure> {
        let info = self.f.files.info(id, &Accessor::Owner).await;
        info.map_err(|e| e.kind)
    }
}

fn starts_with_kind(result: &CallToolResult, kind: FileFailure) {
    assert!(failed(result), "{}", text(result));
    let text = text(result);
    assert!(text.starts_with(kind.as_str()), "{text}");
}

#[tokio::test]
async fn share_references_become_ids_that_the_owner_sees_only_after_sharing() {
    let b = bench().await;
    let id = b.found("photos/2025/IMG 1.jpg", &jpeg(16)).await;
    let result = b.share(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    assert!(text(&result).contains(&id.link()), "{}", text(&result));
    let info = b.owner_info(&id).await.expect("分享后主人能看");
    assert_eq!(info.name, "IMG 1.jpg");
    assert_eq!(
        info.source,
        FileSource::Bot {
            name: AGENT.into(),
            plugin: NOTES.into(),
        }
    );
    let unshared = b.found("photos/2025/IMG 2.jpg", &jpeg(16)).await;
    assert_eq!(
        b.owner_info(&unshared).await.unwrap_err(),
        FileFailure::NotShared
    );
}

#[tokio::test]
async fn the_agent_gets_id_name_type_and_size_but_not_the_reference_or_private_meta() {
    let b = bench().await;
    b.f.put("docs/a.pdf", &[0; 2048]);
    let mut link = Resource::new("wonton-share:docs/a.pdf", "a.pdf");
    link.mime_type = Some("application/pdf".into());
    let mut meta = serde_json::Map::new();
    meta.insert("app.wonton/internal_url".into(), json!("http://x/secret"));
    link.meta = Some(MetaObject(meta));
    let result = b
        .find(vec![
            ContentBlock::text("找到了"),
            ContentBlock::ResourceLink(link),
        ])
        .await;
    let text = text(&result);
    assert!(text.starts_with("找到了"), "其余内容块原样留着：{text}");
    for wanted in ["a.pdf", "application/pdf", "2.0 KiB", "wonton__share_file"] {
        assert!(text.contains(wanted), "{wanted}：{text}");
    }
    let wire = serde_json::to_string(&result).unwrap();
    for leaked in ["wonton-share:", "http://x/secret", "resource_link"] {
        assert!(!wire.contains(leaked), "{leaked}：{wire}");
    }
}

#[tokio::test]
async fn a_second_look_gets_the_same_id() {
    let b = bench().await;
    let first = b.found("docs/a.txt", b"a").await;
    assert_eq!(b.found("docs/a.txt", b"a").await, first);
}

#[tokio::test]
async fn references_that_cannot_be_registered_are_explained_and_private_meta_goes_too() {
    let b = bench().await;
    b.f.put("secret/a.txt", b"a");
    b.f.host
        .revoked_shares
        .lock()
        .unwrap()
        .push("secret".into());
    let mut web = Resource::new("https://example.com/a", "a");
    let mut meta = serde_json::Map::new();
    meta.insert("app.wonton/internal_url".into(), json!("http://y/secret"));
    meta.insert("kept".into(), json!(1));
    web.meta = Some(MetaObject(meta));
    let mut with_url = Resource::new("notes://a", "b.txt");
    let mut meta = serde_json::Map::new();
    meta.insert("app.wonton/internal_url".into(), json!("http://z/secret"));
    with_url.meta = Some(MetaObject(meta));
    let result = b
        .find(vec![
            share_link("secret/a.txt"),
            share_link("docs/../../etc/passwd"),
            ContentBlock::ResourceLink(web),
            ContentBlock::ResourceLink(with_url),
        ])
        .await;
    let wire = serde_json::to_string(&result).unwrap();
    for leaked in ["http://y/secret", "http://z/secret", "wonton-share:"] {
        assert!(!wire.contains(leaked), "{leaked}：{wire}");
    }
    assert!(
        wire.contains("https://example.com/a"),
        "网址原样留着：{wire}"
    );
    assert!(wire.contains("\"kept\":1"), "别的 _meta 原样留着：{wire}");
    assert!(ids(&result).is_empty(), "{wire}");
    let text = text(&result);
    assert!(text.contains("permission_denied"), "{text}");
}

#[tokio::test]
async fn plugins_cannot_fake_the_failure_category() {
    let b = bench().await;
    let mut found = CallToolResult::success(vec![ContentBlock::text("好")]);
    let mut meta = serde_json::Map::new();
    meta.insert("app.wonton/failure".into(), json!("tool_timed_out"));
    found.meta = Some(MetaObject(meta));
    *b.s.notes.found.lock().unwrap() = found;
    let result = b.s.call(FIND, json!({})).await;
    assert!(result.meta.is_none(), "{:?}", result.meta);
}

#[tokio::test]
async fn one_result_registers_at_most_a_hundred_files() {
    let b = bench().await;
    let links: Vec<ContentBlock> = (0..103)
        .map(|i| {
            let path = format!("docs/{i}.txt");
            b.f.put(&path, b"a");
            share_link(&path)
        })
        .collect();
    let result = b.find(links).await;
    assert_eq!(ids(&result).len(), 100);
    assert!(text(&result).contains("还有 3 个"), "{}", text(&result));
}

#[tokio::test]
async fn the_first_four_images_get_previews_no_larger_than_1024() {
    let b = bench().await;
    let mut links = Vec::new();
    for i in 0..6 {
        let path = format!("photos/{i}.jpg");
        b.f.put(&path, &jpeg(1600));
        links.push(share_link(&path));
    }
    b.f.put("photos/broken.jpg", b"not an image");
    links.insert(0, share_link("photos/broken.jpg"));
    let result = b.find(links).await;
    assert_eq!(ids(&result).len(), 7, "解不了的文字照常");
    let previews = images(&result);
    assert_eq!(previews, [(1024, 1024); 3], "第一张解不了，也占一个名额");
}

#[tokio::test]
async fn view_file_returns_a_preview_and_says_why_it_cannot() {
    let b = bench().await;
    let photo = b.found("photos/a.jpg", &jpeg(1600)).await;
    let result = b.view(&photo).await;
    assert!(!failed(&result), "{}", text(&result));
    assert_eq!(images(&result), [(1024, 1024)]);

    let pdf = b.found("docs/a.pdf", b"%PDF").await;
    starts_with_kind(&b.view(&pdf).await, FileFailure::NotViewable);

    let unknown: FileId = "a".repeat(26).parse().unwrap();
    starts_with_kind(&b.view(&unknown).await, FileFailure::FileUnknown);
    let garbage =
        b.s.call("wonton__view_file", json!({"file_id": "../../etc/passwd"}))
            .await;
    starts_with_kind(&garbage, FileFailure::FileUnknown);
    let missing = b.s.call("wonton__share_file", json!({})).await;
    starts_with_kind(&missing, FileFailure::InvalidInput);
}

#[tokio::test]
async fn files_changed_after_registering_must_be_found_again() {
    let b = bench().await;
    let id = b.found("docs/a.png", b"one").await;
    b.f.put("docs/a.png", b"changed");
    starts_with_kind(&b.view(&id).await, FileFailure::FileChanged);
    starts_with_kind(&b.share(&id).await, FileFailure::FileChanged);

    let again = b.found("docs/a.png", b"changed").await;
    assert_eq!(again, id, "同一个引用还是原来的编号");
    assert!(!failed(&b.share(&id).await));

    b.f.put("docs/a.png", b"owner edited");
    assert!(
        b.owner_info(&id).await.is_ok(),
        "主人那边不受影响：改了自己的文档，链接不该失效"
    );
}

#[tokio::test]
async fn gone_files_and_revoked_grants_are_reported() {
    let b = bench().await;
    let id = b.found("docs/a.txt", b"a").await;
    std::fs::remove_file(b.f.shares_dir.join("docs/a.txt")).unwrap();
    starts_with_kind(&b.share(&id).await, FileFailure::FileGone);

    let id = b.found("photos/a.txt", b"a").await;
    assert!(!failed(&b.share(&id).await));
    b.f.host
        .revoked_shares
        .lock()
        .unwrap()
        .push("photos".into());
    starts_with_kind(&b.share(&id).await, FileFailure::PermissionDenied);
    assert!(
        b.owner_info(&id).await.is_ok(),
        "分享以后，主人下载不再依赖来源"
    );
}

#[tokio::test]
async fn bots_cannot_touch_files_outside_their_scope() {
    let b = bench().await;
    let others = b.f.shared_with_bot("docs/theirs.txt", "someone-else");
    starts_with_kind(&b.share(&others).await, FileFailure::FileUnknown);
    let owners = b.f.shared_with_owner("docs/mine.txt", b"a");
    starts_with_kind(&b.view(&owners).await, FileFailure::FileUnknown);
}

/// App 在 Bot 的房间里发链接时调 `grants`，Bot 才能看设备传的文件。
#[tokio::test]
async fn bots_see_device_uploads_once_granted() {
    let b = bench().await;
    let id = b.f.uploaded("a.jpg", &jpeg(32));
    starts_with_kind(&b.view(&id).await, FileFailure::FileUnknown);
    b.f.host
        .rooms
        .lock()
        .unwrap()
        .insert("!bot:home".into(), vec![AGENT.into()]);
    b.f.files.grant_room(&id, "!bot:home").await.unwrap();
    let result = b.view(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    assert_eq!(images(&result), [(32, 32)]);
}

#[tokio::test]
async fn the_audit_log_names_the_file_plugin_and_reference() {
    let b = bench().await;
    let id = b.found("docs/a.txt", b"a").await;
    let result = b.share(&id).await;
    assert!(!failed(&result));
    let audit = b.s.audit();
    let expected = json!({"id": id.as_str(), "plugin": NOTES, "reference": "docs/a.txt"});
    for tool in [FIND, "wonton__share_file"] {
        let line: &Value = audit.iter().find(|l| l["tool"] == tool).unwrap();
        assert_eq!(line["files"], json!([expected]), "{line}");
    }
    starts_with_kind(&b.view(&id).await, FileFailure::NotViewable);
    let refused = b.s.audit().pop().unwrap();
    assert_eq!(refused["outcome"], "not_viewable", "{refused}");
    assert_eq!(refused["decision"], "denied", "{refused}");
}

#[tokio::test]
async fn without_the_file_service_references_are_refused() {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    *s.notes.found.lock().unwrap() = CallToolResult::success(vec![share_link("docs/a.txt")]);
    let result = s.call(FIND, json!({})).await;
    let wire = serde_json::to_string(&result).unwrap();
    assert!(text(&result).contains("service_unavailable"), "{wire}");
    assert!(!wire.contains("wonton-share:"), "{wire}");
    let id = "a".repeat(26);
    let viewed = s.call("wonton__view_file", json!({"file_id": id})).await;
    starts_with_kind(&viewed, FileFailure::ServiceUnavailable);
}
