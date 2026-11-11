//! 上传的行为测试：钩子的鉴权和决定、转发、放文件、清理。tusd 用记下请求头的替身；真的 tusd 见 `tusd_tests`。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use hyper::header::HeaderMap;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use wonton_protocol::FileId;

use super::TunnelPeer;
use super::files_tests::{Home, Reply, home_with, info};
use crate::config::FILE_HOOKS_HOST;
use crate::files::test_support::any_langs;
use crate::plugin::manager::now_secs;

/// 替身：回 tus 的状态码，`Upload-Offset` 是 `offset`，记下最后一次请求的方法和请求头。
#[derive(Clone, Default)]
pub(super) struct FakeTusd {
    offset: Arc<AtomicU64>,
    /// 不是 0 就回这个状态码。
    pub(super) status: Arc<AtomicU64>,
    pub(super) seen: Arc<Mutex<Option<(Method, HeaderMap)>>>,
    /// 创建时回的 `Location`。
    pub(super) location: Arc<Mutex<Option<String>>>,
    /// 回答下一个请求之前先做这件事。
    pub(super) before_reply: Arc<Mutex<Option<futures_util::future::BoxFuture<'static, ()>>>>,
    /// 收到创建请求时做这件事，给的是网关定的编号：钩子问过核心，写了部分数据。
    pub(super) on_create: Arc<Mutex<Option<OnCreate>>>,
}

pub(super) type OnCreate = Box<dyn FnOnce(FileId) + Send>;

impl FakeTusd {
    async fn start(&self) -> SocketAddr {
        let fake = self.clone();
        let app = Router::new().fallback(move |req: Request| {
            let fake = fake.clone();
            async move {
                *fake.seen.lock().unwrap() = Some((req.method().clone(), req.headers().clone()));
                let on_create = fake.on_create.lock().unwrap().take();
                if let Some(on_create) = on_create
                    && let Some(id) = req.headers().get("x-wonton-upload-id")
                {
                    on_create(id.to_str().unwrap().parse().unwrap());
                }
                let before_reply = fake.before_reply.lock().unwrap().take();
                if let Some(before_reply) = before_reply {
                    before_reply.await;
                }
                let status = match *req.method() {
                    Method::POST => StatusCode::CREATED,
                    Method::HEAD => StatusCode::OK,
                    _ => StatusCode::NO_CONTENT,
                };
                let forced = fake.status.load(Ordering::SeqCst);
                let status = StatusCode::from_u16(forced as u16).unwrap_or(status);
                let offset = fake.offset.load(Ordering::SeqCst);
                let location = fake.location.lock().unwrap().clone().unwrap_or_default();
                (
                    status,
                    [
                        ("upload-offset", offset.to_string()),
                        ("location", location),
                    ],
                )
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        addr
    }

    pub(super) fn header(&self, name: &str) -> Option<String> {
        let seen = self.seen.lock().unwrap();
        let (_, headers) = seen.as_ref()?;
        Some(headers.get(name)?.to_str().unwrap().to_owned())
    }

    pub(super) fn calls(&self) -> Option<Method> {
        self.seen.lock().unwrap().as_ref().map(|(m, _)| m.clone())
    }
}

/// 替身和接着它的网关。
pub(super) async fn home() -> (Home, FakeTusd, SocketAddr) {
    let fake = FakeTusd::default();
    let addr = fake.start().await;
    (home_with(Some(addr.to_string())).await, fake, addr)
}

fn hook_body(kind: &str, upload: &Value, device: Option<&str>) -> String {
    let id = crate::files::new_upload_id();
    let header = device.map_or(
        json!({}),
        |device| json!({"X-Wonton-Device": [device], "X-Wonton-Upload-Id": [id]}),
    );
    json!({"Type": kind, "Event": {"Upload": upload, "HTTPRequest": {"Method": "POST", "Header": header}}})
        .to_string()
}

fn create_upload(size: u64) -> Value {
    json!({"Size": size, "MetaData": {"filename": "a.txt", "filetype": "text/plain"}})
}

impl Home {
    /// tusd 的地址发来的钩子。
    pub(super) async fn hook(&self, from: SocketAddr, body: String) -> Reply {
        let request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/")
            .header(hyper::header::HOST, format!("{FILE_HOOKS_HOST}:8080"))
            .extension(ConnectInfo(from))
            .body(Body::from(body))
            .unwrap();
        self.send(request).await
    }

    /// 这台测试设备经钩子创建一个上传，数据已经写好 `content`，返回编号。
    pub(super) async fn pending(&self, tusd: SocketAddr, size: u64, content: &[u8]) -> FileId {
        let device = self.device.0.to_string();
        self.pending_of(&device, tusd, size, content).await
    }

    pub(super) async fn pending_of(
        &self,
        device: &str,
        tusd: SocketAddr,
        size: u64,
        content: &[u8],
    ) -> FileId {
        let body = hook_body("pre-create", &create_upload(size), Some(device));
        let reply = self.hook(tusd, body).await;
        let id: FileId = reply.json()["ChangeFileInfo"]["ID"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        std::fs::write(self.f.uploads_dir().join(id.as_str()), content).unwrap();
        id
    }

    pub(super) fn tus(&self, method: Method, id: &FileId) -> hyper::Request<Body> {
        self.request(method, &format!("/_wonton/files/uploads/{id}"))
            .header("tus-resumable", "1.0.0")
            .body(Body::empty())
            .unwrap()
    }
}

#[tokio::test]
async fn hooks_only_come_from_tusd() {
    let (home, _, tusd) = home().await;
    let body = || hook_body("pre-create", &create_upload(10), Some("phone"));
    let elsewhere: SocketAddr = "192.0.2.1:1234".parse().unwrap();
    let reply = home.hook(elsewhere, body()).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), "permission_denied");

    // 来源地址对了，但它经隧道来。
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri("/")
        .header(hyper::header::HOST, FILE_HOOKS_HOST)
        .extension(ConnectInfo(tusd))
        .extension(TunnelPeer(SecretKey::generate().public()))
        .body(Body::from(body()))
        .unwrap();
    assert_eq!(home.send(request).await.status, StatusCode::FORBIDDEN);

    assert_eq!(home.hook(tusd, body()).await.status, StatusCode::OK);

    let unconfigured = home_with(None).await;
    let reply = unconfigured.hook(tusd, body()).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn creating_an_upload_issues_an_id_that_is_not_a_file_yet() {
    let (home, _, tusd) = home().await;
    let id = home.pending(tusd, 10, b"").await;
    let reply = home.get(&info(&id)).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error_kind(), "file_unknown");
}

#[tokio::test]
async fn creating_an_upload_says_why_it_cannot() {
    let (home, _, tusd) = home().await;
    for (upload, device, status, kind) in [
        (
            create_upload(u64::MAX / 2),
            Some("phone"),
            507,
            "storage_full",
        ),
        (create_upload(u64::MAX), Some("phone"), 507, "storage_full"),
        (
            json!({"SizeIsDeferred": true, "MetaData": {"filename": "a.txt"}}),
            Some("phone"),
            400,
            "invalid_input",
        ),
        (
            json!({"Size": 10, "MetaData": {}}),
            Some("phone"),
            400,
            "invalid_input",
        ),
        // 请求没有经过网关：tusd 只在私有网络里，这是最后一道。
        (create_upload(10), None, 401, "unauthenticated"),
    ] {
        let reply = home
            .hook(tusd, hook_body("pre-create", &upload, device))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "钩子本身是通的");
        let answer = reply.json();
        assert_eq!(answer["RejectUpload"], true, "{upload}");
        let response = &answer["HTTPResponse"];
        assert_eq!(response["StatusCode"], status, "{upload}");
        assert_eq!(response["Header"]["X-Wonton-Error"], kind);
        let body: Value = serde_json::from_str(response["Body"].as_str().unwrap()).unwrap();
        assert_eq!(body["error"]["kind"], kind);
    }
}

#[tokio::test]
async fn uploads_are_for_paired_devices_speaking_tus() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 10, b"").await;
    let path = format!("/_wonton/files/uploads/{id}");
    let base = "/_wonton/files/uploads/";

    let no_files_header = hyper::Request::builder()
        .method(Method::POST)
        .uri(base)
        .header(hyper::header::HOST, format!("{}:18480", super::tests::CORE))
        .header("tus-resumable", "1.0.0")
        .extension(home.device)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        home.send(no_files_header).await.error_kind(),
        "unauthenticated"
    );

    let no_peer = hyper::Request::builder()
        .method(Method::HEAD)
        .uri(&path)
        .header(hyper::header::HOST, format!("{}:18480", super::tests::CORE))
        .header("tus-resumable", "1.0.0")
        .header("x-wonton-files", "1")
        .body(Body::empty())
        .unwrap();
    assert_eq!(home.send(no_peer).await.error_kind(), "unauthenticated");

    for (method, path) in [(Method::POST, base), (Method::PATCH, path.as_str())] {
        let reply = home.call(method, path).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "没带 Tus-Resumable");
        assert_eq!(reply.error_kind(), "invalid_input");
    }

    // 不是核心发的编号，到不了 tusd。
    let reply = home
        .send(home.tus(Method::HEAD, &FileId::from_bytes([1; 16])))
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(fake.calls(), None, "这些请求都没有转发");
}

#[tokio::test]
async fn forwarding_replaces_what_the_client_could_forge() {
    let (home, fake, _) = home().await;
    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "10")
        .header("x-wonton-device", "someone-else")
        .header("x-http-method-override", "DELETE")
        .header("x-forwarded-host", "evil.example")
        .body(Body::empty())
        .unwrap();
    let reply = home.send(request).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(
        reply.header(hyper::header::X_CONTENT_TYPE_OPTIONS),
        "nosniff"
    );
    assert_eq!(
        fake.header("x-wonton-device").as_deref(),
        Some(home.device.0.to_string().as_str())
    );
    assert_eq!(fake.header("x-http-method-override"), None);
    assert_eq!(fake.header("x-wonton-files"), None);
    assert_eq!(
        fake.header("x-forwarded-host").as_deref(),
        Some(format!("{}:18480", super::tests::CORE).as_str())
    );
    assert_eq!(fake.header("tus-resumable").as_deref(), Some("1.0.0"));
}

#[tokio::test]
async fn an_unreachable_or_missing_tusd_is_reported() {
    let home_without = home_with(None).await;
    let tus = |home: &Home| {
        home.request(Method::POST, "/_wonton/files/uploads/")
            .header("tus-resumable", "1.0.0")
            .body(Body::empty())
            .unwrap()
    };
    let reply = home_without.send(tus(&home_without)).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), "service_unavailable");

    let dead = home_with(Some("127.0.0.1:1".to_owned())).await;
    let reply = dead.send(tus(&dead)).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), "service_unavailable");
}

#[tokio::test]
async fn tusd_outside_the_core_network_is_an_upstream_error() {
    let home = home_with(Some("192.0.2.10:8080".into())).await;
    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .body(Body::empty())
        .unwrap();
    let reply = home.send(request).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.error_kind(), "upstream_error");
}

#[tokio::test]
async fn the_last_patch_answers_only_when_the_file_is_available() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;

    fake.offset.store(3, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(
        home.get(&info(&id)).await.error_kind(),
        "file_unknown",
        "还没传完"
    );

    fake.offset.store(5, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let file = home.get(&info(&id)).await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.json()["name"], "a.txt");
    assert_eq!(file.json()["size"], 5);
    assert_eq!(file.json()["source"]["type"], "device");
    assert_eq!(
        std::fs::read(home.f.shares_dir.join("transfers/a.txt")).unwrap(),
        b"hello"
    );
    assert!(!home.f.uploads_dir().join(id.as_str()).exists());

    let line = home.f.audit_lines().pop().unwrap();
    assert_eq!(line["event"], "file_upload");
    assert_eq!(line["file"], id.as_str());
    assert_eq!(line["device"], home.device.0.to_string());
    assert_eq!(line["outcome"], "ok");

    // tusd 那边已经没有它了，进度由核心回答。
    fake.seen.lock().unwrap().take();
    let head = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(head.header("upload-offset"), "5");
    assert_eq!(head.header("upload-length"), "5");
    assert_eq!(head.header("tus-resumable"), "1.0.0");
    assert_eq!(head.header("cache-control"), "no-store");
    assert_eq!(fake.calls(), None);
    let again = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(again.error_kind(), "invalid_input");
    assert_eq!(fake.calls(), None);
}

#[tokio::test]
async fn a_failed_placement_is_reported_and_retried() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;
    let transfers = home.f.shares_dir.join("transfers");
    let aside = home.f.dir.path().join("transfers-aside");
    std::fs::rename(&transfers, &aside).unwrap();
    std::os::unix::fs::symlink(home.f.dir.path(), &transfers).unwrap();

    fake.offset.store(5, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), "permission_denied");
    let line = home.f.audit_lines().pop().unwrap();
    assert_eq!(line["outcome"], "permission_denied");

    std::fs::remove_file(&transfers).unwrap();
    std::fs::rename(&aside, &transfers).unwrap();
    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(home.get(&info(&id)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn post_finish_places_the_file_even_if_the_client_is_gone() {
    let (home, _, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;
    let upload = json!({"ID": id, "Size": 5, "MetaData": {"filename": "a.txt"}});
    let reply = home
        .hook(tusd, hook_body("post-finish", &upload, Some("phone")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    for _ in 0..500 {
        if home.get(&info(&id)).await.status == StatusCode::OK {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("等了 5 秒文件还没放好");
}

#[tokio::test]
async fn data_that_does_not_match_the_declared_length_is_not_a_file() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello, more than declared").await;
    fake.offset.store(5, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), "internal");
    assert!(!home.f.shares_dir.join("transfers/a.txt").exists());
}

/// 记录写不成（磁盘满时就是这样）：文件已经在 `transfers` 里，数据也还在。之后重试收敛到同一个文件，客户端看到的是原因，不是 tusd 的裸错误。
#[tokio::test]
async fn a_record_that_cannot_be_written_converges_on_retry() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;
    let records = home.f.dir.path().join("files/records");
    let saved = home.f.dir.path().join("records-aside");
    std::fs::rename(&records, &saved).unwrap();
    std::fs::write(&records, "").unwrap();

    fake.offset.store(5, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), "internal");
    let transfers = home.f.shares_dir.join("transfers");
    assert!(transfers.join("a.txt").exists());
    assert!(home.f.uploads_dir().join(id.as_str()).exists());

    std::fs::remove_file(&records).unwrap();
    std::fs::rename(&saved, &records).unwrap();
    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(home.get(&info(&id)).await.status, StatusCode::OK);
    let names: Vec<_> = std::fs::read_dir(&transfers)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|name| name != ".wonton-trash")
        .collect();
    assert_eq!(names, ["a.txt"], "不多出一份");
    assert!(!home.f.uploads_dir().join(id.as_str()).exists());
}

/// 记录写好了但没来得及清理：再放一次把数据清掉，不会每小时重放一次。
#[tokio::test]
async fn placing_what_already_has_a_record_clears_the_leftovers() {
    let (home, _, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;
    home.f.files.finish_upload(&id, &any_langs()).await.unwrap();
    let data = home.f.uploads_dir().join(id.as_str());
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    assert!(!pending.exists());
    std::fs::write(&data, "hello").unwrap();
    home.f.files.finish_upload(&id, &any_langs()).await.unwrap();
    assert!(!data.exists());
}

#[tokio::test]
async fn an_upload_belongs_to_the_device_that_created_it() {
    let (home, fake, tusd) = home().await;
    let id = home.pending_of("phone", tusd, 5, b"he").await;
    for method in [Method::HEAD, Method::PATCH, Method::DELETE] {
        let reply = home.send(home.tus(method.clone(), &id)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(reply.error_kind(), "not_shared");
    }
    assert_eq!(fake.calls(), None, "到不了 tusd");
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    assert!(pending.exists());
}

/// 传到一半 tusd 回 500：磁盘真满了才说 `storage_full`，别的 500 不猜。
#[tokio::test]
async fn a_full_disk_midway_is_told_apart_from_other_tusd_errors() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"he").await;
    fake.status.store(500, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), "", "磁盘没满，tusd 的响应原样转");

    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    let text = std::fs::read_to_string(&pending).unwrap();
    let huge = text.replace("\"size\": 5", &format!("\"size\": {}", u64::MAX / 2));
    assert_ne!(text, huge);
    std::fs::write(&pending, huge).unwrap();
    let reply = home.send(home.tus(Method::PATCH, &id)).await;
    assert_eq!(reply.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(reply.error_kind(), "storage_full");
}

/// tusd 没有这个上传了（比如数据被删了）：不转它的裸 404，也不留着记录。
#[tokio::test]
async fn a_pending_upload_tusd_lost_is_forgotten() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"he").await;
    fake.status.store(404, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.error_kind(), "file_unknown");
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    assert!(!pending.exists());
}

#[tokio::test]
async fn cancelling_forgets_the_upload() {
    let (home, _, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hel").await;
    let reply = home.send(home.tus(Method::DELETE, &id)).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.error_kind(), "file_unknown");
}

#[tokio::test]
async fn sweeping_places_finished_uploads_and_clears_abandoned_ones() {
    let (home, _, tusd) = home().await;
    let finished = home.pending(tusd, 5, b"hello").await;
    let partial = home.pending(tusd, 5, b"he").await;
    let uploads = home.f.uploads_dir();
    let orphan = uploads.join("zzzz.info");
    std::fs::write(&orphan, "{}").unwrap();

    home.f.files.sweep(now_secs()).await;
    assert_eq!(home.get(&info(&finished)).await.status, StatusCode::OK);
    assert!(uploads.join(partial.as_str()).exists(), "还在传的不动");
    assert!(orphan.exists(), "不知道是谁的文件，放够久才删");

    let later = now_secs() + crate::files::uploads::STALE_SECS + 60;
    // 把链接发出去，免得传完的这条在同一个 24 小时里到期。
    home.f
        .files
        .send_to_devices(&finished, &any_langs())
        .await
        .unwrap();
    home.f.files.sweep(later).await;
    assert!(!uploads.join(partial.as_str()).exists());
    assert!(!orphan.exists());
    let reply = home.send(home.tus(Method::HEAD, &partial)).await;
    assert_eq!(reply.error_kind(), "file_unknown");
    assert_eq!(
        home.get(&info(&finished)).await.status,
        StatusCode::OK,
        "传完的不受影响"
    );
}

/// 部署文件里的 tusd，只列要用的字段。
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
pub(super) struct TusdService {
    pub(super) image: String,
    pub(super) environment: std::collections::BTreeMap<String, String>,
    pub(super) command: Vec<String>,
    pub(super) read_only: bool,
    pub(super) cap_drop: Vec<String>,
    pub(super) security_opt: Vec<String>,
}

pub(super) fn deployed_tusd(compose: &str) -> TusdService {
    #[derive(serde::Deserialize)]
    struct Compose {
        services: Services,
    }
    #[derive(serde::Deserialize)]
    struct Services {
        tusd: TusdService,
    }
    serde_saphyr::from_str::<Compose>(compose)
        .unwrap()
        .services
        .tusd
}

#[test]
fn both_compose_files_run_the_same_tusd() {
    let without_port = |compose| {
        let mut tusd = deployed_tusd(compose);
        tusd.command
            .retain(|flag| !flag.starts_with("-hooks-http="));
        tusd
    };
    let install = without_port(crate::install::COMPOSE);
    assert_eq!(
        install,
        without_port(include_str!("../../../../deploy/dev/core/compose.yaml"))
    );
    assert!(install.image.contains("@sha256:"), "镜像按摘要固定");
    assert!(install.read_only);
    assert_eq!(install.cap_drop, ["ALL"]);
    assert_eq!(install.security_opt, ["no-new-privileges:true"]);
}

/// 核心接在每个插件网络上，插件的服务名会成为那里的别名。核心访问系统服务用的名字必须带点（服务名不能带点），而且确实是那个服务的别名。
#[test]
fn the_core_reaches_system_services_by_names_plugins_cannot_take() {
    #[derive(serde::Deserialize)]
    struct Compose {
        services: std::collections::BTreeMap<String, Service>,
    }
    #[derive(serde::Deserialize)]
    struct Service {
        #[serde(default)]
        environment: Value,
        #[serde(default)]
        networks: Value,
    }
    for compose in [
        crate::install::COMPOSE,
        include_str!("../../../../deploy/dev/core/compose.yaml"),
    ] {
        let compose: Compose = serde_saphyr::from_str(compose).unwrap();
        let env = &compose.services["core"].environment;
        for (variable, service) in [
            ("WONTON_OIDC_ADMIN_URL", "hydra"),
            ("WONTON_OIDC_PUBLIC_URL", "hydra"),
            ("WONTON_MATRIX_URL", "tuwunel"),
            ("WONTON_TUSD_URL", "tusd"),
        ] {
            let url = url::Url::parse(env[variable].as_str().unwrap()).unwrap();
            let host = url.host_str().unwrap();
            assert!(host.ends_with(".wonton.internal"), "{variable}: {host}");
            let aliases = &compose.services[service].networks["core"]["aliases"];
            assert!(
                aliases.as_array().unwrap().iter().any(|a| a == host),
                "{service} 没有别名 {host}"
            );
        }
    }
}

/// tusd 刚删了数据、核心刚放好：客户端问进度时拿到 404，不能说要重传，否则多出一份 `a (1).txt`。
#[tokio::test]
async fn a_404_from_tusd_right_after_placing_reports_the_file_as_finished() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;
    let (files, task) = (home.f.files.clone(), id.clone());
    *fake.before_reply.lock().unwrap() = Some(Box::pin(async move {
        files.finish_upload(&task, &any_langs()).await.unwrap();
    }));
    fake.status.store(404, Ordering::SeqCst);

    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.header(hyper::header::HeaderName::from_static("upload-offset")),
        "5"
    );
    let transfers = home.f.shares_dir.join("transfers");
    assert_eq!(std::fs::read(transfers.join("a.txt")).unwrap(), b"hello");
    assert!(!transfers.join("a (1).txt").exists());
}

/// 没放好就是数据真的没了：这次上传接不上了。
#[tokio::test]
async fn a_404_from_tusd_before_placing_means_upload_again() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"he").await;
    fake.status.store(404, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.error_kind(), "file_unknown");
    let again = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(again.error_kind(), "file_unknown");
}

/// 取消和放文件排同一个队：放文件时取消要等，取消之后放文件也不会再放。
#[tokio::test]
async fn cancelling_waits_for_placing_and_is_not_interleaved_with_it() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;
    let held = home.f.files.upload_turn(&id).await;
    let cancel = home.send(home.tus(Method::DELETE, &id));
    tokio::pin!(cancel);
    let waiting = tokio::time::timeout(std::time::Duration::from_millis(100), &mut cancel).await;
    assert!(waiting.is_err(), "别人在放的时候不能取消");
    assert_ne!(fake.calls(), Some(Method::DELETE), "tusd 的数据还不能删");
    drop(held);
    assert_eq!(cancel.await.status, StatusCode::NO_CONTENT);
    assert_eq!(home.get(&info(&id)).await.status, StatusCode::NOT_FOUND);
    let placed = home.f.files.finish_upload(&id, &any_langs()).await;
    assert_eq!(
        placed.unwrap_err().kind,
        wonton_protocol::FileFailure::FileUnknown
    );
    assert!(!home.f.shares_dir.join("transfers/a.txt").exists());
}

/// 放文件失败时也要回 tusd 的响应：客户端没有 `Location` 就没法问进度，可之后清理照样会把文件放好。
#[tokio::test]
async fn creating_a_complete_upload_answers_with_its_location_even_if_placing_fails() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 0, b"").await;
    *fake.location.lock().unwrap() = Some(format!("/files/{id}"));
    let transfers = home.f.shares_dir.join("transfers");
    let aside = home.f.dir.path().join("transfers-aside");
    std::fs::rename(&transfers, &aside).unwrap();
    std::os::unix::fs::symlink(home.f.dir.path(), &transfers).unwrap();

    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "0")
        .body(Body::empty())
        .unwrap();
    let reply = home.send(request).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(
        reply.header(hyper::header::LOCATION),
        format!("/files/{id}")
    );

    std::fs::remove_file(&transfers).unwrap();
    std::fs::rename(&aside, &transfers).unwrap();
    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(home.get(&info(&id)).await.status, StatusCode::OK);
}
