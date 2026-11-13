//! 文件测试公用的夹具和断言辅助：假插件环境（[`Bench`]）、引用链接和结果解析。

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolResult, ContentBlock, MetaObject, Resource};
use serde_json::json;
use wonton_protocol::{FileFailure, FileId, FileInfo};

use crate::files::record::Accessor;
use crate::files::test_support::Fixture;
use crate::mcp::tests::{Setup, failed, setup, text};
use crate::plugin::docker::Engine as Docker;

pub(super) const AGENT: &str = "com.example.agent";
pub(super) const NOTES: &str = "app.example.notes";
pub(super) const FIND: &str = "app-example-notes__find";

/// 测试不挑语言：空的 `Languages` 协商成英文。
pub(super) fn any_langs() -> crate::plugin::languages::Languages {
    crate::plugin::languages::Languages::from_header(None)
}

pub(super) struct Bench {
    pub(super) s: Setup,
    pub(super) f: Fixture,
}

pub(super) async fn bench() -> Bench {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    let f = Fixture::new();
    s.bus.attach_files(f.files.clone());
    Bench { s, f }
}

pub(super) fn jpeg(edge: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(edge, edge)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

pub(super) fn share_link(path: &str) -> ContentBlock {
    let name = path.rsplit('/').next().unwrap();
    ContentBlock::ResourceLink(Resource::new(format!("wonton-share:{path}"), name))
}

pub(super) fn images(result: &CallToolResult) -> Vec<(u32, u32)> {
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
pub(super) fn ids(result: &CallToolResult) -> Vec<FileId> {
    text(result)
        .split("id ")
        .skip(1)
        .map(|rest| rest[..26].parse().unwrap())
        .collect()
}

pub(super) fn plugin_resource(uri: &str, name: &str, internal_url: Option<&str>) -> ContentBlock {
    let mut link = Resource::new(uri, name);
    if let Some(url) = internal_url {
        let mut meta = serde_json::Map::new();
        meta.insert("app.wonton/internal_url".into(), json!(url));
        link.meta = Some(MetaObject(meta));
    }
    ContentBlock::ResourceLink(link)
}

impl Bench {
    /// 假插件在运行，只有一个叫 `web` 的服务。
    pub(super) fn plugin_runs(&self) {
        self.f.host.services.lock().unwrap().insert(
            NOTES.to_owned(),
            [("web".to_owned(), "127.0.0.1:1".to_owned())].into(),
        );
    }

    pub(super) async fn find(&self, blocks: Vec<ContentBlock>) -> CallToolResult {
        *self.s.notes.found.lock().unwrap() = CallToolResult::success(blocks);
        self.s.call(FIND, json!({})).await
    }

    pub(super) async fn found(&self, path: &str, content: &[u8]) -> FileId {
        self.f.put(path, content);
        let result = self.find(vec![share_link(path)]).await;
        ids(&result).pop().expect("应该登记成编号")
    }

    pub(super) async fn view(&self, id: &FileId) -> CallToolResult {
        self.s
            .call("wonton__view_file", json!({"file_id": id.as_str()}))
            .await
    }

    /// 经内网地址登记一个插件来源的文件（走真的 HTTP 到本机假服务）。
    pub(super) async fn plugin_file(&self, port: u16, name: &str) -> FileId {
        self.f.host.services.lock().unwrap().insert(
            NOTES.to_owned(),
            [("web".to_owned(), format!("127.0.0.1:{port}"))].into(),
        );
        let result = self
            .find(vec![plugin_resource(
                &format!("notes://{name}"),
                name,
                Some("http://web/a.txt"),
            )])
            .await;
        ids(&result).pop().expect("应该登记成编号")
    }

    pub(super) async fn share(&self, id: &FileId) -> CallToolResult {
        self.s
            .call("wonton__share_file", json!({"file_id": id.as_str()}))
            .await
    }

    pub(super) async fn owner_info(&self, id: &FileId) -> Result<FileInfo, FileFailure> {
        let info = self.f.files.info(id, &Accessor::Owner, &any_langs()).await;
        info.map_err(|e| e.kind)
    }
}

pub(super) fn starts_with_kind(result: &CallToolResult, kind: FileFailure) {
    assert!(failed(result), "{}", text(result));
    let text = text(result);
    assert!(text.starts_with(kind.as_str()), "{text}");
}
