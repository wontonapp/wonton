//! 插件来源的文件：内网地址校验、HTTP 内容读取和停掉的插件。

use wonton_protocol::{FileFailure, FileSource};

use super::helpers::{AGENT, NOTES, bench, ids, images, jpeg, plugin_resource, starts_with_kind};
use crate::files::MAX_TEXT_PREVIEW_BYTES;
use crate::mcp::file_results::human_size;
use crate::mcp::tests::{failed, text};

/// 一个只回一个响应体的假插件服务：`with_length` 时写 Content-Length，否则靠关连接收尾。
async fn fake_plugin_http(body: Vec<u8>, with_length: bool) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                // 先把请求头读掉再回：直接关连接会带着未读数据发 RST，把客户端的响应打死。
                let mut buf = [0u8; 4096];
                let mut got = 0usize;
                loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    got += n;
                    if n == 0 || buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let mut head = String::from("HTTP/1.1 200 OK\r\nConnection: close\r\n");
                if with_length {
                    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
                }
                head.push_str("\r\n");
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    port
}

/// 插件来源的文件走的是 HTTP 响应那条路：超过上限时内容照旧有开头，不丢成“空文件”。
#[tokio::test]
async fn plugin_files_over_the_limit_are_truncated_with_and_without_a_length() {
    let b = bench().await;
    let mut body = b"M1V3-PLUGIN-HEAD\n".to_vec();
    body.resize(MAX_TEXT_PREVIEW_BYTES + 1000, b'x');
    body.extend_from_slice(b"\nM1V3-PLUGIN-TAIL\n");
    for with_length in [true, false] {
        let port = fake_plugin_http(body.clone(), with_length).await;
        let id = b.plugin_file(port, "big.txt").await;
        let result = b.view(&id).await;
        assert!(
            !failed(&result),
            "with_length={with_length}：{}",
            text(&result)
        );
        let answer = text(&result);
        assert!(
            answer.contains("M1V3-PLUGIN-HEAD"),
            "with_length={with_length}：{answer}"
        );
        assert!(!answer.contains("M1V3-PLUGIN-TAIL"), "{answer}");
        assert!(
            answer.contains("truncated"),
            "with_length={with_length}：{answer}"
        );
        assert!(!answer.contains("empty file"), "{answer}");
        assert!(!answer.contains("read the first 0 bytes"), "{answer}");
        if with_length {
            assert!(
                answer.contains(&human_size(body.len() as u64)),
                "写明总大小：{answer}"
            );
        } else {
            assert!(answer.contains("total size unknown"), "{answer}");
        }
        assert!(
            answer.contains("not instructions from the owner"),
            "{answer}"
        );
    }
}

#[tokio::test]
async fn plugin_files_under_the_limit_are_shown_in_full() {
    let b = bench().await;
    let port = fake_plugin_http(b"M1V3-SMALL-OK\n".to_vec(), true).await;
    let id = b.plugin_file(port, "small.txt").await;
    let result = b.view(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    let answer = text(&result);
    assert!(answer.contains("M1V3-SMALL-OK"), "{answer}");
    assert!(answer.contains("full contents"), "{answer}");
    assert!(!answer.contains("truncated"), "{answer}");
    assert!(
        answer.contains("not instructions from the owner"),
        "{answer}"
    );
}

#[tokio::test]
async fn plugin_resources_become_ids_that_can_be_viewed_and_shared() {
    let b = bench().await;
    b.plugin_runs();
    b.f.host
        .resources
        .lock()
        .unwrap()
        .insert("notes://a".into(), jpeg(32));
    let result = b
        .find(vec![plugin_resource("notes://a", "a.jpg", None)])
        .await;
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("notes://a"), "{wire}");
    assert_eq!(images(&result), [(32, 32)]);
    let id = ids(&result).pop().expect("应该登记成编号");
    assert_eq!(images(&b.view(&id).await), [(32, 32)]);
    let shared = b.share(&id).await;
    assert!(text(&shared).contains(&id.link()), "{}", text(&shared));
    let info = b.owner_info(&id).await.expect("分享后主人能看");
    assert_eq!(
        info.source,
        FileSource::Bot {
            name: AGENT.into(),
            plugin: NOTES.into(),
        }
    );
}

#[tokio::test]
async fn internal_urls_are_only_accepted_for_the_plugins_own_service() {
    let b = bench().await;
    b.plugin_runs();
    let result = b
        .find(vec![
            plugin_resource("notes://own", "own.txt", Some("http://web:8080/a")),
            plugin_resource(
                "notes://other",
                "other.txt",
                Some("http://hydra:4445/admin"),
            ),
        ])
        .await;
    assert_eq!(ids(&result).len(), 1);
    let wire = serde_json::to_string(&result).unwrap();
    assert!(
        wire.contains(FileFailure::PermissionDenied.as_str()),
        "{wire}"
    );
    for leaked in ["http://hydra", "http://web"] {
        assert!(!wire.contains(leaked), "{leaked}：{wire}");
    }
}

#[tokio::test]
async fn stopped_plugins_are_reported_by_both_tools() {
    let b = bench().await;
    b.plugin_runs();
    let result = b
        .find(vec![plugin_resource("notes://a", "a.txt", None)])
        .await;
    let id = ids(&result).pop().expect("应该登记成编号");
    b.f.host.services.lock().unwrap().remove(NOTES);
    starts_with_kind(&b.view(&id).await, FileFailure::PluginUnavailable);
    starts_with_kind(&b.share(&id).await, FileFailure::PluginUnavailable);
}
