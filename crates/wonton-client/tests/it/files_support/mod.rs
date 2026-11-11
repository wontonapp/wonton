//! 文件行为测试的替身：本机的隧道服务端、假的核心文件接口和 tus。

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::{StreamExt, stream};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use tempfile::TempDir;
use tokio::net::TcpListener;
use wonton_client::{
    Action, ClientCore, FileFailure, FileId, FileInfo, FileProblem, FileSource, HomeStatus,
    Network, Transfer,
};
use wonton_tunnel::{Allowlist, TunnelServer};

const WAIT: Duration = Duration::from_secs(60);

pub fn id(seed: u8) -> FileId {
    FileId::from_bytes([seed; 16])
}

/// 每个位置都不一样的内容：错位、重传、漏传都比得出来。
pub fn source_bytes(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i % 251) as u8 ^ (i >> 16) as u8)
        .collect()
}

pub fn read_opened(path: &FsPath) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[derive(Default)]
pub struct Upload {
    pub length: u64,
    pub bytes: Vec<u8>,
    pub metadata: HashMap<String, String>,
}

#[derive(Clone)]
pub struct Served {
    name: String,
    bytes: Vec<u8>,
    etag: String,
    thumbnail: Result<Vec<u8>, FileFailure>,
}

impl Served {
    pub fn new(name: &str, bytes: &[u8], etag: &str) -> Self {
        Self {
            name: name.to_owned(),
            bytes: bytes.to_vec(),
            etag: format!("\"{etag}\""),
            thumbnail: Err(FileFailure::NotViewable),
        }
    }
}

/// 假的核心：记下收到的请求，按设定回答。
#[derive(Default)]
pub struct Core {
    pub uploads: HashMap<String, Upload>,
    pub files: HashMap<String, Served>,
    pub grants: Vec<(String, String)>,
    pub sent: Vec<String>,
    pub deleted: Vec<String>,
    pub terminated: Vec<String>,
    pub info_requests: usize,
    /// 每个 PATCH 的起点。
    pub patches: Vec<u64>,
    /// 每次取内容的 `Range` 和 `If-Range`。
    pub ranges: Vec<(Option<String>, Option<String>)>,
    pub storage_full: bool,
    pub delete_fails_with: Option<FileFailure>,
    pub grant_fails_with: Option<FileFailure>,
    /// 整套服务重启时：核心回带类别的 503。
    pub services_down: bool,
    /// 只有 PATCH 回 503：`HEAD` 走得通，才测得到 PATCH。
    pub patches_down: bool,
    /// 回了几次这种 503。
    pub refused: usize,
    /// 授权一直不回答：字节传完了，停在发链接这一步。
    pub hold_grant: bool,
    /// 下一个 PATCH 收到这么多字节以后不再读，也不回答。
    pub hold_patch_after: Option<usize>,
    /// 下一次下载发这么多字节以后不再发。
    pub hold_download_after: Option<usize>,
    next_id: u8,
}

impl Core {
    pub fn serve(&mut self, name: &str, bytes: &[u8], etag: &str) -> FileId {
        self.next_id += 1;
        let file = id(self.next_id);
        self.files
            .insert(file.to_string(), Served::new(name, bytes, etag));
        file
    }

    pub fn serve_two(&mut self) -> (FileId, FileId) {
        (
            self.serve("a.mp4", b"abc", "v1"),
            self.serve("b.mp4", b"abc", "v1"),
        )
    }

    pub fn thumbnail(&mut self, file: &FileId, thumbnail: Result<Vec<u8>, FileFailure>) {
        self.files.get_mut(file.as_str()).unwrap().thumbnail = thumbnail;
    }

    fn refuse(&mut self) -> Response {
        self.refused += 1;
        failure(
            FileFailure::ServiceUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
        )
    }

    pub fn uploaded_bytes(&self) -> usize {
        self.uploads.values().map(|upload| upload.bytes.len()).sum()
    }
}

#[derive(Clone, Default)]
pub struct SharedCore(Arc<Mutex<Core>>);

impl SharedCore {
    pub fn lock(&self) -> MutexGuard<'_, Core> {
        self.0.lock().unwrap()
    }
}

fn failure(kind: FileFailure, status: StatusCode) -> Response {
    let body = serde_json::json!({"error": {"kind": kind.as_str(), "message": "假的核心说不行"}});
    (
        status,
        [("x-wonton-error", kind.as_str())],
        axum::Json(body),
    )
        .into_response()
}

fn unknown() -> Response {
    failure(FileFailure::FileUnknown, StatusCode::NOT_FOUND)
}

/// 所有文件接口都要带的请求头。
async fn require_files_header(request: Request, next: Next) -> Response {
    if request
        .headers()
        .get("x-wonton-files")
        .map(|v| v.as_bytes())
        != Some(b"1")
    {
        return failure(FileFailure::Unauthenticated, StatusCode::UNAUTHORIZED);
    }
    next.run(request).await
}

fn metadata(headers: &HeaderMap) -> HashMap<String, String> {
    let raw = headers
        .get("upload-metadata")
        .map_or("", |v| v.to_str().unwrap());
    raw.split(',')
        .filter_map(|pair| pair.split_once(' '))
        .map(|(key, value)| {
            let decoded = STANDARD.decode(value).unwrap();
            (key.to_owned(), String::from_utf8(decoded).unwrap())
        })
        .collect()
}

async fn create(State(core): State<SharedCore>, headers: HeaderMap) -> Response {
    let mut core = core.lock();
    if core.storage_full {
        return failure(FileFailure::StorageFull, StatusCode::INSUFFICIENT_STORAGE);
    }
    core.next_id += 1;
    let file = id(core.next_id).to_string();
    let length = headers["upload-length"].to_str().unwrap().parse().unwrap();
    let upload = Upload {
        length,
        bytes: Vec::new(),
        metadata: metadata(&headers),
    };
    core.uploads.insert(file.clone(), upload);
    let location = format!("http://wonton.localhost/_wonton/files/uploads/{file}");
    (StatusCode::CREATED, [(header::LOCATION, location)]).into_response()
}

async fn progress(State(core): State<SharedCore>, Path(file): Path<String>) -> Response {
    let mut core = core.lock();
    if core.services_down {
        return core.refuse();
    }
    let Some(upload) = core.uploads.get(&file) else {
        // tusd 转发过来的 404 不写类别。
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        StatusCode::OK,
        [
            ("upload-offset", upload.bytes.len().to_string()),
            ("upload-length", upload.length.to_string()),
        ],
    )
        .into_response()
}

async fn append(
    State(core): State<SharedCore>,
    Path(file): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let offset: u64 = headers["upload-offset"].to_str().unwrap().parse().unwrap();
    let down = {
        let core = core.lock();
        core.services_down || core.patches_down
    };
    if down {
        // 读完再回：不读就回，服务端关连接时客户端可能先看到写失败，这一次就说不出原因，等下次重试才说得出。
        let _ = axum::body::to_bytes(body, usize::MAX).await;
        return core.lock().refuse();
    }
    let hold = {
        let mut core = core.lock();
        let Some(upload) = core.uploads.get(&file) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if upload.bytes.len() as u64 != offset {
            return StatusCode::CONFLICT.into_response();
        }
        core.patches.push(offset);
        core.hold_patch_after.take()
    };
    let mut stream = body.into_data_stream();
    let mut received = 0;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let keep = hold.map_or(chunk.len(), |hold| chunk.len().min(hold - received));
        core.lock()
            .uploads
            .get_mut(&file)
            .unwrap()
            .bytes
            .extend_from_slice(&chunk[..keep]);
        received += keep;
        if hold == Some(received) {
            std::future::pending::<()>().await;
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn terminate(State(core): State<SharedCore>, Path(file): Path<String>) -> Response {
    let mut core = core.lock();
    core.uploads.remove(&file);
    core.terminated.push(file);
    StatusCode::NO_CONTENT.into_response()
}

async fn info(State(core): State<SharedCore>, Path(file): Path<String>) -> Response {
    let mut core = core.lock();
    if core.services_down {
        return core.refuse();
    }
    core.info_requests += 1;
    let Some(served) = core.files.get(&file) else {
        return unknown();
    };
    axum::Json(FileInfo {
        id: file.parse().unwrap(),
        name: served.name.clone(),
        mime_type: None,
        size: Some(served.bytes.len() as u64),
        source: FileSource::Device,
        has_thumbnail: served.thumbnail.is_ok(),
    })
    .into_response()
}

async fn remove(State(core): State<SharedCore>, Path(file): Path<String>) -> Response {
    let mut core = core.lock();
    if let Some(kind) = core.delete_fails_with {
        return failure(kind, StatusCode::CONFLICT);
    }
    core.files.remove(&file);
    core.deleted.push(file);
    StatusCode::NO_CONTENT.into_response()
}

async fn thumbnail(State(core): State<SharedCore>, Path(file): Path<String>) -> Response {
    let core = core.lock();
    let Some(served) = core.files.get(&file) else {
        return unknown();
    };
    match &served.thumbnail {
        Ok(bytes) => bytes.clone().into_response(),
        Err(kind) => {
            let status = StatusCode::from_u16(kind.http_status().unwrap()).unwrap();
            failure(*kind, status)
        }
    }
}

async fn grant(
    State(core): State<SharedCore>,
    Path(file): Path<String>,
    axum::Json(request): axum::Json<serde_json::Value>,
) -> Response {
    let room = request["room"].as_str().unwrap().to_owned();
    if core.lock().hold_grant {
        std::future::pending::<()>().await;
    }
    let mut core = core.lock();
    core.grants.push((file, room));
    if let Some(kind) = core.grant_fails_with {
        return failure(kind, StatusCode::SERVICE_UNAVAILABLE);
    }
    axum::Json(serde_json::json!({"bots": []})).into_response()
}

async fn send_to_devices(State(core): State<SharedCore>, Path(file): Path<String>) -> Response {
    core.lock().sent.push(file);
    StatusCode::NO_CONTENT.into_response()
}

fn text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    Some(headers.get(name)?.to_str().unwrap().to_owned())
}

fn range_start_end(range: &str, len: usize) -> (usize, usize) {
    let (start, end) = range
        .strip_prefix("bytes=")
        .unwrap()
        .split_once('-')
        .unwrap();
    let start = start.parse().unwrap();
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse().unwrap()
    };
    (start, end)
}

async fn content(
    State(core): State<SharedCore>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    let range = text(&headers, header::RANGE);
    let if_range = text(&headers, header::IF_RANGE);
    let (served, hold) = {
        let mut core = core.lock();
        if core.services_down {
            return core.refuse();
        }
        core.ranges.push((range.clone(), if_range.clone()));
        let Some(served) = core.files.get(&file).cloned() else {
            return unknown();
        };
        (served, core.hold_download_after.take())
    };
    let len = served.bytes.len();
    let matches = if_range.is_none_or(|etag| etag == served.etag);
    let chosen = range.filter(|_| matches).map(|r| range_start_end(&r, len));
    let (status, start, end) = match chosen {
        Some((start, end)) => (StatusCode::PARTIAL_CONTENT, start, end),
        None => (StatusCode::OK, 0, len.saturating_sub(1)),
    };
    let part = if len == 0 {
        Vec::new()
    } else {
        served.bytes[start..=end].to_vec()
    };
    let mut builder = Response::builder()
        .status(status)
        .header(header::ETAG, served.etag)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, part.len());
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
    }
    let body = match hold {
        None => Body::from(part),
        Some(hold) => {
            let first = Bytes::copy_from_slice(&part[..hold.min(part.len())]);
            let held = stream::iter([Ok::<_, std::io::Error>(first)]).chain(stream::pending());
            Body::from_stream(held)
        }
    };
    builder.body(body).unwrap()
}

async fn serve_core(core: SharedCore) -> SocketAddr {
    let router = Router::new()
        .route("/_wonton/files/uploads/", post(create))
        .route(
            "/_wonton/files/uploads/{id}",
            axum::routing::head(progress)
                .patch(append)
                .delete(terminate),
        )
        .route("/_wonton/files/{id}", get(info).merge(delete(remove)))
        .route("/_wonton/files/{id}/content", get(content))
        .route("/_wonton/files/{id}/thumbnail", get(thumbnail))
        .route("/_wonton/files/{id}/grants", post(grant))
        .route("/_wonton/files/{id}/send-to-devices", post(send_to_devices))
        .layer(middleware::from_fn(require_files_header))
        .with_state(core);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    addr
}

async fn loopback_endpoint(key: SecretKey, port: u16) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, port))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// 一台手机（有自己的数据目录）和一台家里的服务器。
pub struct World {
    pub core: SharedCore,
    dir: TempDir,
    phone: Option<Arc<ClientCore>>,
    server: Option<TunnelServer>,
    key: SecretKey,
    allowlist: Allowlist,
    upstream: SocketAddr,
    ticket: String,
}

fn open_phone(dir: &FsPath) -> Arc<ClientCore> {
    ClientCore::with_network(dir.join("phone").to_str().unwrap(), Network::DirectOnly).unwrap()
}

impl World {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("phone")).unwrap();
        std::fs::create_dir_all(dir.path().join("files")).unwrap();
        let phone = open_phone(dir.path());
        let device: EndpointId = phone.home().device_id.parse().unwrap();
        let core = SharedCore::default();
        let upstream = serve_core(core.clone()).await;
        let key = SecretKey::generate();
        let allowlist = Allowlist::new([device]);
        let endpoint = loopback_endpoint(key.clone(), 0).await;
        let server = TunnelServer::spawn(endpoint, allowlist.clone(), upstream);
        let (id, addr) = (server.endpoint().id(), server.endpoint().bound_sockets()[0]);
        let ticket = format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
                "base_domain": "wonton.localhost", "web_url": "http://wonton.localhost/"}}"#
        );
        let world = Self {
            core,
            dir,
            phone: Some(phone),
            server: Some(server),
            key,
            allowlist,
            upstream,
            ticket,
        };
        world
            .phone()
            .dispatch(Action::ConnectHome {
                ticket: world.ticket.clone(),
            })
            .unwrap();
        world.wait_connected().await;
        world
    }

    pub fn phone(&self) -> &ClientCore {
        self.phone.as_ref().expect("手机没在运行")
    }

    async fn wait_connected(&self) {
        let phone = self.phone();
        self.wait_until(|| {
            let view = phone.home();
            view.status == HomeStatus::Connected && view.proxy.is_some()
        })
        .await;
    }

    /// App 被杀。
    pub fn stop_phone(&mut self) {
        self.phone.take();
    }

    pub async fn start_phone(&mut self) {
        self.phone = Some(open_phone(self.dir.path()));
        self.wait_connected().await;
    }

    /// 家里的服务器没了时打开手机：连不上，正好试离线的卡片。
    pub async fn start_phone_without_home(&mut self) {
        self.phone = Some(open_phone(self.dir.path()));
    }

    pub async fn restart_phone(&mut self) {
        self.stop_phone();
        self.start_phone().await;
    }

    /// 不发关闭帧就丢掉所有连接，再用同样的身份和端口启动。
    pub async fn drop_connections(&mut self) {
        let port = self.stop_server();
        self.start_server(port).await;
    }

    /// 不发关闭帧就丢掉所有连接，返回端口，交给 [`start_server`](Self::start_server)。
    pub fn stop_server(&mut self) -> u16 {
        let server = self.server.take().expect("服务端没在运行");
        server.endpoint().bound_sockets()[0].port()
    }

    pub async fn start_server(&mut self, port: u16) {
        let endpoint = loop {
            // 端口可能还没放开。
            let attempt = Endpoint::builder(presets::Minimal)
                .secret_key(self.key.clone())
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, port))
                .unwrap()
                .bind()
                .await;
            if let Ok(endpoint) = attempt {
                break endpoint;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        self.server = Some(TunnelServer::spawn(
            endpoint,
            self.allowlist.clone(),
            self.upstream,
        ));
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join("files").join(name)
    }

    /// App 下载目录里这个文件的路径。
    pub fn downloaded(&self, file: &FileId, name: &str) -> PathBuf {
        self.dir
            .path()
            .join("phone/servers/1/downloads")
            .join(file.as_str())
            .join(name)
    }

    /// App 更新后容器的路径变了：把数据目录挪走，从新的位置打开。
    pub fn move_phone_data(&mut self, to: &str) -> (Arc<ClientCore>, PathBuf) {
        self.stop_phone();
        let moved = self.dir.path().join(to);
        std::fs::rename(self.dir.path().join("phone"), &moved).unwrap();
        let phone = ClientCore::with_network(moved.to_str().unwrap(), Network::DirectOnly).unwrap();
        (phone, moved)
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    pub async fn wait_until(&self, mut done: impl FnMut() -> bool) {
        let waiting = async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "等待超时，传输列表：{:?}",
                    self.phone.as_ref().map(|p| p.transfers())
                )
            });
    }

    pub async fn wait_core(&self, mut done: impl FnMut(&Core) -> bool) {
        let core = self.core.clone();
        self.wait_until(move || done(&core.lock())).await;
    }

    /// 等列表里有一条满足条件。
    pub async fn wait_transfer(&self, done: impl Fn(&Transfer) -> bool) -> Transfer {
        let subscription = self.phone().subscribe_transfers();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(transfer) = view.all().find(|transfer| done(transfer)) {
                    return transfer.clone();
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，传输列表：{:?}", self.phone().transfers()))
    }

    pub async fn wait_named(&self, name: &str, done: impl Fn(&Transfer) -> bool) -> Transfer {
        let subscription = self.phone().subscribe_transfers();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                let found = view.all().find(|t| t.name == name && done(t));
                if let Some(transfer) = found {
                    return transfer.clone();
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，传输列表：{:?}", self.phone().transfers()))
    }

    pub async fn wait_problem(&self) -> FileProblem {
        let subscription = self.phone().subscribe_transfers();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(problem) = view.problem {
                    return problem;
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(WAIT, waiting).await.expect("等待超时")
    }
}
