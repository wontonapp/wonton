//! 插件资源的文件接口：内网地址（只跟它自己的服务、不跟随跳转）和 `resources/read`。

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::body::Body;
use axum::routing::get;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use tokio::net::TcpListener;

use super::files_tests::{content, home};
use crate::files::record::{Accessor, Origin};
use crate::plugin::PluginId;

/// 假插件的内网服务：`/file` 支持 `Range`，`/plain` 不支持，`/moved` 跳转，`/broken` 报错。
pub(super) async fn plugin_service() -> SocketAddr {
    async fn file(headers: HeaderMap) -> axum::response::Response {
        let mut builder = axum::response::Response::builder()
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::ETAG, "\"plugin-tag\"")
            .header(header::CONTENT_TYPE, "text/html");
        let range = headers.get(header::RANGE).map(|r| r.to_str().unwrap());
        if range.is_some_and(|range| range.contains(',')) {
            let multipart = "--x\r\nContent-Range: bytes 0-0/4\r\n\r\na\r\n--x--\r\n";
            return builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, "multipart/byteranges; boundary=x")
                .body(Body::from(multipart))
                .unwrap();
        }
        if range == Some("bytes=9-") {
            return builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, "bytes */4")
                .body(Body::empty())
                .unwrap();
        }
        if range.is_some() {
            builder = builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, "bytes 1-2/4")
                .header(header::CONTENT_LENGTH, "2");
            return builder.body(Body::from("bc")).unwrap();
        }
        builder.body(Body::from("abcd")).unwrap()
    }
    let app = Router::new()
        .route("/file", get(file))
        .route("/plain", get(|| async { "plain" }))
        .route(
            "/moved",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "http://hydra:4445/admin")],
                    "",
                )
            }),
        )
        .route(
            "/broken",
            get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "磁盘坏了") }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

pub(super) fn plugin_origin(path: &str) -> Origin {
    Origin::Plugin {
        plugin: PluginId::parse("app.immich").unwrap(),
        uri: "immich://a".into(),
        internal_url: Some(format!("http://server:2283{path}")),
    }
}

#[tokio::test]
async fn plugin_resources_come_from_their_internal_url() {
    let home = home().await;
    let service = plugin_service().await;
    home.f.host.services.lock().unwrap().insert(
        "app.immich".into(),
        [("server".to_owned(), service.to_string())].into(),
    );
    let access = || [Accessor::Owner].into();

    let id = home.f.save("a.pdf", plugin_origin("/file"), access());
    let whole = home.get(&content(&id)).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert_eq!(whole.body, b"abcd");
    assert_eq!(whole.header(header::ACCEPT_RANGES), "bytes");
    assert_eq!(whole.header(header::ETAG), "\"plugin-tag\"");
    assert_eq!(
        whole.header(header::CONTENT_TYPE),
        "application/octet-stream",
        "类型由核心决定，不听插件的"
    );
    assert_eq!(whole.header(header::CONTENT_SECURITY_POLICY), "sandbox");
    let part = home.range(&content(&id), "bytes=1-2", None).await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body, b"bc");
    assert_eq!(part.header(header::CONTENT_RANGE), "bytes 1-2/4");

    // 多段的不转：插件的 multipart 响应配上文件的类型，客户端收到的内容就坏了。
    let several = home.range(&content(&id), "bytes=0-0,2-3", None).await;
    assert_eq!(several.status, StatusCode::OK);
    assert_eq!(several.body, b"abcd");

    let plain = home.f.save("a.txt", plugin_origin("/plain"), access());
    let reply = home.get(&content(&plain)).await;
    assert_eq!(reply.body, b"plain");
    assert_eq!(
        reply.header(header::ACCEPT_RANGES),
        "",
        "不支持 Range 的不回"
    );

    let moved = home.f.save("a.txt", plugin_origin("/moved"), access());
    let reply = home.get(&content(&moved)).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.error_kind(), "upstream_error");
    assert_eq!(reply.header(header::LOCATION), "");

    let broken = home.f.save("a.txt", plugin_origin("/broken"), access());
    let reply = home.get(&content(&broken)).await;
    assert_eq!(reply.error_kind(), "upstream_error");
    let message = reply.json()["error"]["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(message.contains("磁盘坏了"), "附上原话：{message}");

    // 记录里的地址不是它自己的服务（比如插件升级后去掉了这个服务）。
    let other = Origin::Plugin {
        plugin: PluginId::parse("app.immich").unwrap(),
        uri: "immich://a".into(),
        internal_url: Some("http://hydra:4445/admin".into()),
    };
    let id = home.f.save("a.txt", other, access());
    let reply = home.get(&content(&id)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), "permission_denied");

    home.f.host.services.lock().unwrap().clear();
    let reply = home.get(&content(&plain)).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), "plugin_unavailable");
}

#[tokio::test]
async fn resources_without_an_internal_url_are_read_over_mcp() {
    let home = home().await;
    home.f
        .host
        .services
        .lock()
        .unwrap()
        .insert("app.immich".into(), Default::default());
    home.f
        .host
        .resources
        .lock()
        .unwrap()
        .insert("immich://a".into(), b"from mcp".to_vec());
    let origin = Origin::Plugin {
        plugin: PluginId::parse("app.immich").unwrap(),
        uri: "immich://a".into(),
        internal_url: None,
    };
    let id = home.f.save("a.txt", origin, [Accessor::Owner].into());
    let reply = home.get(&content(&id)).await;
    assert_eq!(reply.body, b"from mcp");
    assert_eq!(reply.header(header::ACCEPT_RANGES), "");
    assert_eq!(reply.header(header::CONTENT_LENGTH), "8");

    // 插件声明了大小，问大小就不用把整个资源读一遍。
    home.f.declare_size(&id, 8);
    let reads = home.f.host.resource_reads.load(Ordering::SeqCst);
    let head = home.call(Method::HEAD, &content(&id)).await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(head.header(header::CONTENT_LENGTH), "8");
    assert_eq!(home.f.host.resource_reads.load(Ordering::SeqCst), reads);
}
