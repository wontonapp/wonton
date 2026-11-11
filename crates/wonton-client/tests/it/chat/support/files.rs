//! 假核心里的文件服务接口，只够「传完文件发链接」的测试用：tus 的创建、问进度、传字节，加上文件信息和授权。
//! tus 的回答要带 `Location`、`Upload-Offset` 这些头，通用应答写不了，在这里自己写。

use std::sync::atomic::Ordering;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use super::{Core, FakeFile, read_body};

/// 文件传完的测试要的两样都在这里：上传（编号 → 内容）和授权（文件 → 房间）。认下请求就自己写应答。
pub(super) async fn serve(core: &Core, text: &str, raw: &str, tcp: &mut TcpStream) -> bool {
    if text.starts_with("post /_wonton/files/uploads/ ") {
        create(core, raw, tcp).await;
        return true;
    }
    for (method, path) in [
        ("head", "head /_wonton/files/uploads/"),
        ("patch", "patch /_wonton/files/uploads/"),
    ] {
        if text.starts_with(path) {
            let id = file_id(text, path);
            tus(core, method, &id, raw, tcp).await;
            return true;
        }
    }
    if text.starts_with("post /_wonton/files/") && text.contains("/grants ") {
        grant(core, text, tcp).await;
        return true;
    }
    if text.starts_with("get /_wonton/files/")
        && !text.contains("/thumbnail ")
        && !text.contains("/content ")
    {
        info(core, file_id(text, "get /_wonton/files/"), tcp).await;
        return true;
    }
    false
}

/// 创建上传：记下长度和名字，发回客户端认得的编号。
async fn create(core: &Core, raw: &str, tcp: &mut TcpStream) {
    let length = header_value(raw, "upload-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let name = header_value(raw, "upload-metadata")
        .and_then(|metadata| metadata.split(',').find_map(metadata_value("filename ")))
        .unwrap_or_default();
    let count = core.next_file.fetch_add(1, Ordering::SeqCst);
    // 编号得是客户端认的 26 个小写 base32（128 位）：第一位的字母往上数，测试里够分。
    let first = char::from(b'a' + u8::try_from(count % 26).unwrap_or(0));
    let id = format!("{}{}", first, "a".repeat(25));
    core.files.lock().unwrap().insert(
        id.clone(),
        FakeFile {
            length,
            bytes: Vec::new(),
            name,
        },
    );
    reply(
        tcp,
        201,
        &[(
            "location",
            format!("http://wonton.localhost/_wonton/files/uploads/{id}"),
        )],
        b"",
    )
    .await;
}

/// 问进度和传字节：回答里带 tus 的 `Upload-Offset`。
async fn tus(core: &Core, method: &str, id: &str, raw: &str, tcp: &mut TcpStream) {
    let body = if method == "patch" {
        read_body(tcp, raw).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    let offset: u64 = header_value(raw, "upload-offset")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let answer = {
        let mut core = core.files.lock().unwrap();
        match core.get_mut(id) {
            None => (404, Vec::new()),
            Some(_) if method == "head" => (200, Vec::new()),
            Some(file) if offset != file.bytes.len() as u64 => (409, Vec::new()),
            Some(file) => {
                file.bytes.extend_from_slice(&body);
                (204, Vec::new())
            }
        }
    };
    let at = {
        let core = core.files.lock().unwrap();
        core.get(id).map_or(0, |file| file.bytes.len())
    };
    let headers = match answer.0 {
        200 => vec![
            ("upload-offset", at.to_string()),
            ("upload-length", {
                let core = core.files.lock().unwrap();
                core.get(id).map_or(0, |file| file.length).to_string()
            }),
        ],
        204 => vec![("upload-offset", at.to_string())],
        _ => Vec::new(),
    };
    reply(tcp, answer.0, &headers, &answer.1).await;
}

/// 把这个房间的 Bot 加进文件的访问范围。
async fn grant(core: &Core, text: &str, tcp: &mut TcpStream) {
    let file = text
        .strip_prefix("post /_wonton/files/")
        .unwrap()
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let Some(body) = read_body(tcp, text).await else {
        return;
    };
    let room: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!(null));
    let room = room["room"].as_str().unwrap_or_default().to_owned();
    core.file_grants.lock().unwrap().push((file, room));
    reply(tcp, 200, &[], br#"{"bots": []}"#).await;
}

/// 文件信息：名字来自创建时的 `Upload-Metadata`。
async fn info(core: &Core, file_id: String, tcp: &mut TcpStream) {
    let answer = {
        let core = core.files.lock().unwrap();
        match core.get(&file_id) {
            None => (
                404,
                br#"{"error": {"kind": "file_unknown", "message": "no such file"}}"#.to_vec(),
            ),
            Some(file) => {
                let info = json!({
                    "id": file_id,
                    "name": file.name,
                    "mime_type": serde_json::Value::Null,
                    "size": file.bytes.len(),
                    "source": {"type": "device"},
                    "has_thumbnail": false,
                });
                (200, info.to_string().into_bytes())
            }
        }
    };
    reply(tcp, answer.0, &[], &answer.1).await;
}

/// 请求行里路径的第一段（编号）。
fn file_id(text: &str, prefix: &str) -> String {
    text.strip_prefix(prefix)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|path| path.split('/').next())
        .unwrap_or_default()
        .to_owned()
}

fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .map(str::trim)
        .map(str::to_owned)
}

/// `Upload-Metadata` 里的一段：`名字 空格 base64`。
fn metadata_value(prefix: &'static str) -> impl Fn(&str) -> Option<String> {
    move |pair: &str| {
        pair.strip_prefix(prefix)
            .and_then(|value| STANDARD.decode(value).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }
}

async fn reply(tcp: &mut TcpStream, status: u16, headers: &[(&str, String)], body: &[u8]) {
    let mut response = format!(
        "HTTP/1.1 {status} Whatever\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    tcp.write_all(response.as_bytes()).await.ok();
    if !body.is_empty() {
        tcp.write_all(body).await.ok();
    }
    tcp.shutdown().await.ok();
}
