//! 假核心的链接预览接口（ADR-0065）：每个网址的应答是测试配的（[`Home::set_link_preview`]），图片按 id
//! 给回字节。图片是二进制，和字符串应答的接口分开，自己写应答。

use serde_json::Value;
use std::sync::{Arc, atomic::Ordering};

use percent_encoding::percent_decode_str;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use super::{Core, Home};

/// 预览和图片的路由都从这里走：处理了返回真。
pub(super) async fn serve(core: &Arc<Core>, text: &str, tcp: &mut TcpStream) -> bool {
    if let Some(rest) = text.strip_prefix("get /_wonton/api/link-preview/images/") {
        let id = rest.split_whitespace().next().unwrap_or_default();
        let image = core.link_preview_images.lock().unwrap().get(id).cloned();
        match image {
            Some((mime_type, bytes)) => write(tcp, 200, &mime_type, &bytes).await,
            None => write(tcp, 404, "application/json", b"{}").await,
        }
        return true;
    }
    if text.starts_with("get /_wonton/api/link-preview?")
        || text.starts_with("get /_wonton/api/link-preview ")
    {
        core.link_preview_requests.fetch_add(1, Ordering::SeqCst);
        let url = query(text, "url").unwrap_or_default();
        let reply = core.link_previews.lock().unwrap().get(&url).cloned();
        match reply {
            Some((status, body)) => write(tcp, status, "application/json", body.as_bytes()).await,
            None => write(tcp, 404, "application/json", b"{}").await,
        }
        return true;
    }
    false
}

async fn write(tcp: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} Whatever\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n",
        body.len()
    );
    tcp.write_all(head.as_bytes()).await.ok();
    tcp.write_all(body).await.ok();
    tcp.shutdown().await.ok();
}

/// 查询串里的一个参数，百分号解开；本地代理一个请求一行头，参数就在请求行里。
fn query(text: &str, name: &str) -> Option<String> {
    let line = text.lines().next()?;
    let start = line.find(&format!("{name}="))? + name.len() + 1;
    let rest = &line[start..];
    let end = rest.find([' ', '&']).unwrap_or(rest.len());
    let decoded = percent_decode_str(&rest[..end]).decode_utf8().ok()?;
    Some(decoded.into_owned())
}

impl Home {
    /// 配一个网址的预览应答：`status` 和核心会回的 `body`（成功是预览 JSON，失败是错误 JSON）。
    pub fn set_link_preview(&self, url: &str, status: u16, body: Value) {
        self.core
            .link_previews
            .lock()
            .unwrap()
            .insert(url.to_owned(), (status, body.to_string()));
    }

    /// 配一张预览图片：按 id 给回类型和字节。
    pub fn set_link_preview_image(&self, id: &str, mime_type: &str, bytes: &[u8]) {
        self.core
            .link_preview_images
            .lock()
            .unwrap()
            .insert(id.to_owned(), (mime_type.to_owned(), bytes.to_vec()));
    }

    /// 取预览的请求到了几次；图片的请求不算，断言“没向核心请求”用它。
    pub fn link_preview_requests(&self) -> usize {
        self.core.link_preview_requests.load(Ordering::SeqCst)
    }
}
