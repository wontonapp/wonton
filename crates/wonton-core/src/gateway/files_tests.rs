//! 文件接口的行为测试：进程内调用网关，设备身份放进请求扩展，和隧道入口一样。真的隧道见 `tests/tusd.rs`。

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, Request, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::{FileId, FileInfo, FileSource};

use super::files_plugin_tests::{plugin_origin, plugin_service};
use super::tests::{CORE, TOKEN, owner};
use super::tunnel_tests::loopback_tunnel;
use super::{Gateway, TunnelPeer, router};
use crate::config::PublicUrls;
use crate::files::record::{Accessor, Origin};
use crate::files::test_support::Fixture;
use crate::files::test_support::any_langs;
use crate::http::HttpClient;
use crate::owner::OwnerStore;
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::tunnel::Tunnel;

pub(super) struct Home {
    pub(super) app: Router,
    pub(super) tunnel: Tunnel,
    pub(super) device: TunnelPeer,
    pub(super) f: Fixture,
}

pub(super) async fn home() -> Home {
    home_with(None).await
}

/// `tusd` 是 tusd 的 `host:port`。
pub(super) async fn home_with(tusd: Option<String>) -> Home {
    let f = Fixture::new();
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: 18480,
    };
    let dir = f.dir.path();
    let http = HttpClient::new();
    let profile = OwnerStore::for_test(dir, &owner());
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.to_owned(),
        apps_dir: dir.join("apps"),
        shares_dir: dir.join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    let manager = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        Store::open(dir.join("plugins")).unwrap(),
        http.clone(),
        None,
        lifecycle,
        ImageWorker::in_process(),
    ));
    let tunnel = loopback_tunnel(dir).await;
    let key = SecretKey::generate();
    tunnel.devices.add(key.public(), None).unwrap();
    let gateway = Gateway::new(
        public,
        profile,
        TOKEN.into(),
        manager,
        None,
        http,
        tunnel.clone(),
    )
    .with_files(Some(f.files.clone()))
    .with_tusd(tusd);
    Home {
        app: router(Arc::new(gateway)),
        tunnel,
        device: TunnelPeer(key.public()),
        f,
    }
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn error_kind(&self) -> &str {
        self.headers
            .get("x-wonton-error")
            .map_or("", |v| v.to_str().unwrap())
    }

    pub(super) fn header(&self, name: impl header::AsHeaderName) -> &str {
        self.headers.get(name).map_or("", |v| v.to_str().unwrap())
    }

    pub(super) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

impl Home {
    pub(super) async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: body.to_vec(),
        }
    }

    /// 已配对设备带着请求头发的请求。
    pub(super) fn request(&self, method: Method, path: &str) -> hyper::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, format!("{CORE}:18480"))
            .header("x-wonton-files", "1")
            .extension(self.device)
    }

    pub(super) async fn call(&self, method: Method, path: &str) -> Reply {
        self.send(self.request(method, path).body(Body::empty()).unwrap())
            .await
    }

    pub(super) async fn get(&self, path: &str) -> Reply {
        self.call(Method::GET, path).await
    }

    pub(super) async fn range(&self, path: &str, range: &str, if_range: Option<&str>) -> Reply {
        let mut request = self.request(Method::GET, path).header(header::RANGE, range);
        if let Some(if_range) = if_range {
            request = request.header(header::IF_RANGE, if_range);
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }

    async fn post(&self, path: &str, body: &Value) -> Reply {
        let request = self
            .request(Method::POST, path)
            .body(Body::from(body.to_string()))
            .unwrap();
        self.send(request).await
    }
}

pub(super) fn content(id: &FileId) -> String {
    format!("/_wonton/files/{id}/content")
}

pub(super) fn info(id: &FileId) -> String {
    format!("/_wonton/files/{id}")
}

#[tokio::test]
async fn files_are_only_for_paired_devices_that_ask_explicitly() {
    let home = home().await;
    let id = home.f.shared_with_owner("photos/a.jpg", b"photo");
    let paths = [info(&id), content(&id), format!("{}/thumbnail", info(&id))];

    let host = format!("{CORE}:18480");
    let unknown = TunnelPeer(SecretKey::generate().public());
    for path in &paths {
        let bare = |builder: hyper::http::request::Builder| {
            builder
                .uri(path.as_str())
                .header(header::HOST, &host)
                .body(Body::empty())
                .unwrap()
        };
        // 局域网浏览器、插件网络：没有隧道身份，带了请求头也不行。
        let lan = bare(Request::builder().header("x-wonton-files", "1"));
        // 隧道身份随 WebView 的请求自动带上：`<img>`、导航不带自定义请求头。
        let webview = bare(Request::builder().extension(home.device));
        let wrong_value = bare(
            Request::builder()
                .extension(home.device)
                .header("x-wonton-files", "true"),
        );
        // 没配对的设备连不上隧道，这里防的是吊销和配对之间的缝。
        let not_paired = bare(
            Request::builder()
                .extension(unknown)
                .header("x-wonton-files", "1"),
        );
        for request in [lan, webview, wrong_value, not_paired] {
            let reply = home.send(request).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(reply.error_kind(), "unauthenticated");
            assert_eq!(reply.json()["error"]["kind"], "unauthenticated");
            assert_eq!(reply.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        }
    }
    assert_eq!(home.get(&info(&id)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn preflights_get_no_cors_headers() {
    let home = home().await;
    let id = home.f.shared_with_owner("photos/a.jpg", b"photo");
    let request = Request::builder()
        .method(Method::OPTIONS)
        .uri(content(&id))
        .header(header::HOST, format!("{CORE}:18480"))
        .header(header::ORIGIN, "http://app-evil.wonton.localhost:18480")
        .header("access-control-request-method", "GET")
        .header("access-control-request-headers", "x-wonton-files")
        .extension(home.device)
        .body(Body::empty())
        .unwrap();
    let reply = home.send(request).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(
        !reply
            .headers
            .keys()
            .any(|k| k.as_str().starts_with("access-control-")),
        "{:?}",
        reply.headers
    );
}

#[tokio::test]
async fn revoking_a_device_takes_effect_on_the_next_request() {
    let home = home().await;
    let id = home.f.shared_with_owner("photos/a.jpg", b"photo");
    assert_eq!(home.get(&info(&id)).await.status, StatusCode::OK);
    home.tunnel.devices.remove(&home.device.0).unwrap();
    assert_eq!(home.get(&info(&id)).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn info_reports_what_the_file_is_now() {
    let home = home().await;
    let id = home.f.shared_with_owner("photos/a.jpg", b"photo");
    let reply = home.get(&info(&id)).await;
    let parsed: FileInfo = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(parsed.id, id);
    assert_eq!(parsed.name, "a.jpg");
    assert_eq!(parsed.size, Some(5));
    assert_eq!(parsed.mime_type.as_deref(), Some("image/jpeg"));
    assert_eq!(parsed.source, FileSource::Device);
    assert!(parsed.has_thumbnail);
    assert_eq!(reply.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");

    home.f.put("photos/a.jpg", b"longer now");
    let reply = home.get(&info(&id)).await;
    assert_eq!(reply.json()["size"], 10);
}

#[tokio::test]
async fn errors_say_why() {
    let home = home().await;
    let unknown = FileId::from_bytes([7; 16]);
    let reply = home.get(&info(&unknown)).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), "file_unknown");
    assert_eq!(reply.json()["error"]["kind"], "file_unknown");

    let malformed = home.get("/_wonton/files/..%2f..%2fetc%2fpasswd").await;
    assert_eq!(malformed.error_kind(), "file_unknown");

    let bots_only = home.f.shared_with_bot("photos/b.jpg", "hermes");
    for path in [info(&bots_only), content(&bots_only)] {
        let reply = home.get(&path).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(reply.error_kind(), "not_shared");
    }
    assert!(home.f.shares_dir.join("photos/b.jpg").exists());
    let reply = home.call(Method::DELETE, &info(&bots_only)).await;
    assert_eq!(reply.error_kind(), "not_shared");

    let gone = home.f.shared_with_owner("photos/c.jpg", b"photo");
    std::fs::remove_file(home.f.shares_dir.join("photos/c.jpg")).unwrap();
    for path in [info(&gone), content(&gone)] {
        let reply = home.get(&path).await;
        assert_eq!(reply.status, StatusCode::GONE, "{path}");
        assert_eq!(reply.error_kind(), "file_gone");
    }
}

#[tokio::test]
async fn a_symlink_swapped_in_after_registration_is_refused() {
    let home = home().await;
    let id = home.f.shared_with_owner("photos/2025/a.jpg", b"photo");
    let dir = home.f.shares_dir.join("photos/2025");
    let outside = home.f.dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("a.jpg"), "secret").unwrap();
    std::fs::rename(&dir, home.f.dir.path().join("moved")).unwrap();
    std::os::unix::fs::symlink(&outside, &dir).unwrap();
    let reply = home.get(&content(&id)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), "permission_denied");
    assert!(!reply.body.starts_with(b"secret"));
}

#[tokio::test]
async fn downloads_support_ranges_and_strong_etags() {
    let home = home().await;
    let bytes: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
    let id = home.f.shared_with_owner("videos/a.mp4", &bytes);
    let path = content(&id);

    let whole = home.get(&path).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert_eq!(whole.body, bytes);
    assert_eq!(whole.header(header::CONTENT_LENGTH), "200000");
    assert_eq!(whole.header(header::ACCEPT_RANGES), "bytes");
    assert_eq!(whole.header(header::CONTENT_TYPE), "video/mp4");
    let etag = whole.header(header::ETAG).to_owned();
    assert!(
        etag.starts_with('"') && etag.ends_with('"'),
        "强 ETag：{etag}"
    );

    let head = home.call(Method::HEAD, &path).await;
    assert_eq!(head.status, StatusCode::OK);
    assert!(head.body.is_empty());
    assert_eq!(head.header(header::CONTENT_LENGTH), "200000");
    assert_eq!(head.header(header::ETAG), etag);

    let part = home.range(&path, "bytes=70000-70009", None).await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body, bytes[70000..70010]);
    assert_eq!(
        part.header(header::CONTENT_RANGE),
        "bytes 70000-70009/200000"
    );
    assert_eq!(part.header(header::CONTENT_LENGTH), "10");

    let tail = home.range(&path, "bytes=199990-", Some(&etag)).await;
    assert_eq!(tail.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(tail.body, bytes[199_990..]);

    let past = home.range(&path, "bytes=200000-", None).await;
    assert_eq!(past.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(past.header(header::CONTENT_RANGE), "bytes */200000");
}

/// 续传中文件变了：`ETag` 和 `If-Range` 对不上，从头下（ADR-0020 的“生命周期”）。
#[tokio::test]
async fn a_file_changed_during_a_resume_restarts_from_the_beginning() {
    let home = home().await;
    let id = home.f.shared_with_owner("docs/a.bin", b"first version");
    let etag = home
        .get(&content(&id))
        .await
        .header(header::ETAG)
        .to_owned();
    home.f.put("docs/a.bin", b"second version!");
    let reply = home.range(&content(&id), "bytes=5-", Some(&etag)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"second version!");
    assert_ne!(reply.header(header::ETAG), etag);
}

#[tokio::test]
async fn only_media_is_served_with_its_type_the_rest_is_a_sandboxed_attachment() {
    let home = home().await;
    for (path, expected_type, inline) in [
        ("media/a.jpg", "image/jpeg", true),
        ("media/a.mp3", "audio/mpeg", true),
        ("media/a.html", "application/octet-stream", false),
        ("media/a.svg", "application/octet-stream", false),
        ("media/a.pdf", "application/octet-stream", false),
    ] {
        let id = home.f.shared_with_owner(path, b"<script>alert(1)</script>");
        let reply = home.get(&content(&id)).await;
        assert_eq!(reply.header(header::CONTENT_TYPE), expected_type, "{path}");
        assert_eq!(reply.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(reply.header(header::CONTENT_SECURITY_POLICY), "sandbox");
        let disposition = reply.header(header::CONTENT_DISPOSITION);
        assert_eq!(disposition.is_empty(), inline, "{path}: {disposition}");
        if !inline {
            assert!(disposition.starts_with("attachment; filename*=UTF-8''"));
        }
    }
    // 上传时声明的类型不算数。
    let id = home.f.save(
        "evil.html",
        Origin::Share {
            reference: crate::files::share_fs::ShareRef::parse("wonton-share:media/a.html")
                .unwrap(),
        },
        [Accessor::Owner].into(),
    );
    let reply = home.get(&content(&id)).await;
    assert_eq!(
        reply.header(header::CONTENT_TYPE),
        "application/octet-stream"
    );
}

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::RgbImage::new(width, height)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

#[tokio::test]
async fn thumbnails_are_small_jpegs_without_the_original_metadata() {
    let home = home().await;
    let mut photo = jpeg(1600, 1000);
    // JPEG 的第一段后面塞一段 COM，里面写上“拍摄地点”：缩略图不能带出去。
    let comment = b"\xFF\xFE\x00\x10GPS-SECRET-HERE!";
    photo.splice(2..2, comment.iter().copied());
    let id = home.f.shared_with_owner("photos/a.jpg", &photo);
    let reply = home.get(&format!("{}/thumbnail", info(&id))).await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{:?}",
        String::from_utf8_lossy(&reply.body)
    );
    assert_eq!(reply.header(header::CONTENT_TYPE), "image/jpeg");
    assert_eq!(reply.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    let thumbnail = image::load_from_memory(&reply.body).unwrap();
    assert_eq!((thumbnail.width(), thumbnail.height()), (800, 500));
    assert!(!reply.body.windows(10).any(|w| w == b"GPS-SECRET"));

    // 小图不放大。
    let id = home.f.shared_with_owner("photos/small.jpg", &jpeg(100, 50));
    let reply = home.get(&format!("{}/thumbnail", info(&id))).await;
    let thumbnail = image::load_from_memory(&reply.body).unwrap();
    assert_eq!((thumbnail.width(), thumbnail.height()), (100, 50));
}

#[tokio::test]
async fn files_without_a_thumbnail_say_so() {
    let home = home().await;
    let id = home.f.shared_with_owner("docs/a.pdf", b"%PDF");
    let reply = home.get(&format!("{}/thumbnail", info(&id))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), "not_viewable");

    // 名字说是图片，内容不是。
    let id = home.f.shared_with_owner("photos/fake.jpg", b"not an image");
    let reply = home.get(&format!("{}/thumbnail", info(&id))).await;
    assert_eq!(reply.error_kind(), "not_viewable");
}

#[tokio::test]
async fn grants_add_the_bots_of_a_room() {
    let home = home().await;
    let id = home.f.shared_with_owner("photos/a.jpg", b"photo");
    home.f
        .host
        .rooms
        .lock()
        .unwrap()
        .insert("!room:home".into(), vec!["hermes".into()]);
    let path = format!("{}/grants", info(&id));
    let reply = home.post(&path, &json!({"room": "!room:home"})).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({"bots": ["hermes"]}));
    assert!(
        home.f
            .files
            .authorize(&id, &Accessor::Bot("hermes".into()), &any_langs())
            .is_ok()
    );

    let bot = format!("{path}/hermes");
    assert_eq!(
        home.call(Method::DELETE, &bot).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(
        home.f
            .files
            .authorize(&id, &Accessor::Bot("hermes".into()), &any_langs())
            .is_err()
    );
    assert_eq!(
        home.call(Method::DELETE, &bot).await.status,
        StatusCode::NO_CONTENT
    );
    home.post(&path, &json!({"room": "!room:home"})).await;

    // 普通聊天里发链接：没有 Bot，什么都不加。
    let reply = home.post(&path, &json!({"room": "!people:home"})).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({"bots": []}));

    let reply = home.post(&path, &json!({"wrong": 1})).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_kind(), "invalid_input");

    // 主人自己看不了的文件，不能转授给 Bot。
    let bots_only = home.f.shared_with_bot("photos/b.jpg", "other");
    let path = format!("{}/grants", info(&bots_only));
    let reply = home.post(&path, &json!({"room": "!room:home"})).await;
    assert_eq!(reply.error_kind(), "not_shared");
    assert!(
        home.f
            .files
            .authorize(&bots_only, &Accessor::Bot("hermes".into()), &any_langs())
            .is_err()
    );
}

#[tokio::test]
async fn sending_to_devices_posts_only_the_link() {
    let home = home().await;
    let id = home.f.shared_with_owner("photos/a.jpg", b"photo");
    let path = format!("{}/send-to-devices", info(&id));
    let reply = home.post(&path, &json!({})).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(*home.f.host.notices.lock().unwrap(), [id.link()]);

    home.f.host.notices_broken.store(true, Ordering::SeqCst);
    let reply = home.post(&path, &json!({})).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), "service_unavailable");

    let bots_only = home.f.shared_with_bot("photos/b.jpg", "hermes");
    let reply = home
        .post(&format!("{}/send-to-devices", info(&bots_only)), &json!({}))
        .await;
    assert_eq!(reply.error_kind(), "not_shared");
}

#[tokio::test]
async fn delete_verifies_the_upload_is_still_the_same_file() {
    let home = home().await;
    let id = home.f.uploaded("a.txt", b"mine");
    let path = home.f.shares_dir.join("transfers/a.txt");
    assert_eq!(
        home.call(Method::DELETE, &info(&id)).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(!path.exists());
    assert_eq!(home.get(&info(&id)).await.error_kind(), "file_unknown");

    let id = home.f.uploaded("a.txt", b"mine");
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, "the owner's other file").unwrap();
    let reply = home.call(Method::DELETE, &info(&id)).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error_kind(), "file_changed");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "the owner's other file"
    );
    assert_eq!(home.get(&info(&id)).await.error_kind(), "file_unknown");
}

/// 416 没有传字节：不算下载成功，也不说内容是什么。
#[tokio::test]
async fn a_range_past_the_end_is_not_a_download() {
    let home = home().await;
    let service = plugin_service().await;
    home.f.host.services.lock().unwrap().insert(
        "app.immich".into(),
        [("server".to_owned(), service.to_string())].into(),
    );
    let local = home.f.uploaded("a.jpg", b"0123");
    let remote = home
        .f
        .save("b.jpg", plugin_origin("/file"), [Accessor::Owner].into());
    for id in [&local, &remote] {
        let reply = home.range(&content(id), "bytes=9-", None).await;
        assert_eq!(reply.status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(reply.header(header::CONTENT_TYPE), "");
        assert_eq!(reply.header(header::CONTENT_DISPOSITION), "");
        assert_eq!(reply.header(header::CONTENT_SECURITY_POLICY), "sandbox");
    }
    let lines = home.f.audit_lines();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        assert_eq!(line["event"], "file_download");
        assert_eq!(line["outcome"], "range_not_satisfiable");
        assert!(line["range"].is_null());
    }
}

#[tokio::test]
async fn transfers_and_changes_are_audited() {
    let home = home().await;
    let id = home.f.uploaded("a.txt", b"0123456789");
    home.range(&content(&id), "bytes=2-5", None).await;
    home.call(Method::HEAD, &content(&id)).await;
    let denied = home.f.shared_with_bot("docs/b.txt", "hermes");
    home.get(&content(&denied)).await;
    home.f
        .host
        .rooms
        .lock()
        .unwrap()
        .insert("!room:home".into(), vec!["hermes".into()]);
    home.post(
        &format!("{}/grants", info(&id)),
        &json!({"room": "!room:home"}),
    )
    .await;
    home.post(&format!("{}/send-to-devices", info(&id)), &json!({}))
        .await;
    home.call(Method::DELETE, &info(&id)).await;

    let device = home.device.0.to_string();
    let lines = home.f.audit_lines();
    let brief: Vec<_> = lines
        .iter()
        .map(|line| {
            assert_eq!(line["device"], device.as_str());
            assert!(line["ts"].is_u64());
            (
                line["event"].as_str().unwrap(),
                line["file"].as_str().unwrap().to_owned(),
                line["outcome"].as_str().unwrap(),
            )
        })
        .collect();
    let id = id.to_string();
    assert_eq!(
        brief,
        [
            ("file_download", id.clone(), "ok"),
            ("file_download", denied.to_string(), "not_shared"),
            ("file_grant", id.clone(), "ok"),
            ("file_send_to_devices", id.clone(), "ok"),
            ("file_delete", id.clone(), "ok"),
        ]
    );
    assert_eq!(lines[0]["range"], "2-5");
    assert_eq!(lines[2]["bots"], json!(["hermes"]));
}

/// 文件接口的错误按 `Accept-Language` 挑语言：ja 和 fr-FR 拿到不同的同一句话（期望值由目录格式化出来）。
#[tokio::test]
async fn file_errors_follow_the_requested_language() {
    use crate::files::new_upload_id;

    let home = home().await;
    let path = info(&new_upload_id());
    let message = {
        let home = &home;
        move |header: &'static str| {
            let path = path.clone();
            async move {
                let reply = home
                    .send(
                        home.request(Method::GET, &path)
                            .header(header::ACCEPT_LANGUAGE, header)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await;
                assert_eq!(reply.error_kind(), "file_unknown", "{header}");
                reply.json()["error"]["message"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }
        }
    };
    let ja = message("ja").await;
    let french = message("fr-FR").await;
    let expected = |tag: &str| {
        crate::l10n::CATALOG.format(
            crate::l10n::CATALOG.negotiate(&[tag]),
            "files-not-found",
            &[],
        )
    };
    assert_eq!(ja, expected("ja"));
    assert_eq!(french, expected("en"));
    assert_ne!(ja, french);
}
