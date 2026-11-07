//! 真的 tusd、真的隧道：确认 ADR-0020 里 tusd 接得上的几点。tusd 用部署时的镜像和参数，开在主机网络上（只有 Linux 上
//! 才连得回测试进程），所以只在 Linux 上编译。

#![cfg(target_os = "linux")]

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use axum::body::{Body, Bytes};
use futures_util::stream;
use http_body_util::BodyExt;
use hyper::{Method, Response, StatusCode};
use iroh::SecretKey;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use wonton_protocol::FileId;

use super::files_tests::{Home, home_at, home_with};
use super::files_upload_tests::deployed_tusd;
use super::tests::CORE;
use super::tunnel_tests::Phone;
use crate::config::FILE_HOOKS_HOST;
use crate::files::test_support::Fixture;
use crate::http::HttpClient;
use crate::test_docker::Names;
use crate::tunnel::TunnelEntry;

const CHUNK: usize = 256 << 10;

/// tusd 退出、容器被 `--rm` 收掉以后，`docker logs` 说的话。
const GONE: [&str; 2] = ["No such container", "can not get logs from container"];

/// 此刻没人监听的端口：绑 0 拿到号码就放掉。只给 `a_tusd_that_is_not_running_shows_as_unavailable` 用，它要的就是没人监听。
/// 别拿它当“安全地把端口交给别的进程”的办法：放掉到对方绑上之间，别人能抢走，那正是这里修掉的问题。
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// 退出时删掉的 tusd 容器：名字由调用方的 test_docker::Names 登记并回收。
struct Tusd {
    name: String,
    port: u16,
}

impl Tusd {
    /// 镜像和参数取自部署文件，端口交给 tusd 自己挑（`-port=0`，内核在 bind 时原子分配）。
    fn start(uploads: &Path, gateway: u16, name: String) -> Self {
        let service = deployed_tusd(crate::install::COMPOSE);
        let meta = std::fs::metadata(uploads).unwrap();
        let mut docker = Command::new("docker");
        docker
            .args(["run", "-d", "--rm", "--network", "host"])
            .args(["--name", &name])
            .args(["--add-host", &format!("{FILE_HOOKS_HOST}:127.0.0.1")])
            .args(["--user", &format!("{}:{}", meta.uid(), meta.gid())])
            .args(service.read_only.then_some("--read-only"))
            .args(
                service
                    .cap_drop
                    .iter()
                    .flat_map(|cap| ["--cap-drop".to_owned(), cap.clone()]),
            )
            .args(
                service
                    .security_opt
                    .iter()
                    .flat_map(|opt| ["--security-opt".to_owned(), opt.clone()]),
            )
            .args(["-v", &format!("{}:/srv/uploads", uploads.display())])
            .args(
                service
                    .environment
                    .iter()
                    .flat_map(|(name, value)| ["--env".to_owned(), format!("{name}={value}")]),
            )
            .arg(&service.image)
            .args(["-host=127.0.0.1", "-port=0"]);
        for flag in service.command {
            docker.arg(if flag.starts_with("-hooks-http=") {
                format!("-hooks-http=http://{FILE_HOOKS_HOST}:{gateway}/")
            } else {
                flag
            });
        }
        let output = docker.output().expect("运行 docker");
        assert!(
            output.status.success(),
            "起不了 tusd：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self { name, port: 0 }
    }

    /// 等 tusd 报出它挑上的端口并且确实在服务，`self.port` 填上那个端口；监听没建成（端口被占）返回 false。
    ///
    /// tusd 先建监听、再打 “You can now upload files to: http://127.0.0.1:端口/…”（`listener.Addr()`），
    /// 所以这行出来时端口已经绑好了。拿到端口再发一个 OPTIONS 核对回话的确实是 tusd（`Tus-Version`），
    /// 不是“端口连得上”就算数。
    async fn wait_ready(&mut self) -> bool {
        for _ in 0..300 {
            let Some(logs) = self.logs() else {
                return false;
            };
            if let Some(port) = listening_port(&logs) {
                if self.is_tusd(port).await {
                    self.port = port;
                    return true;
                }
            } else if logs.contains("Unable to create listener") {
                return false;
            } else if GONE.iter().any(|gone| logs.contains(gone)) {
                // 容器已经没了，日志不会再变。
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "等了 30 秒 tusd 还没开始监听：{}",
            self.logs().unwrap_or_default()
        );
    }

    /// 回这个端口的确实是 tusd：OPTIONS 会带 `Tus-Version`，别的进程不会。
    async fn is_tusd(&self, port: u16) -> bool {
        let request = hyper::Request::builder()
            .method(Method::OPTIONS)
            .uri(format!("http://127.0.0.1:{port}/_wonton/files/uploads/"))
            .header("tus-resumable", "1.0.0")
            .body(Body::empty())
            .unwrap();
        HttpClient::new()
            .send(request)
            .await
            .is_ok_and(|response| response.headers().get("tus-version").is_some())
    }

    /// 容器日志。容器已经被 `--rm` 收掉时取不到（`None`）：起不来、退出以后就是这样。
    fn logs(&self) -> Option<String> {
        let output = Command::new("docker").args(["logs", &self.name]).output();
        let output = output.ok()?;
        output.status.success().then(|| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }
}

struct Setup {
    home: Home,
    phone: Phone,
    tusd: Tusd,
    /// 守卫活到测试结束，tusd 容器随它回收。
    _names: Names,
    _entry: TunnelEntry,
}

async fn setup() -> Setup {
    let mut names = Names::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_port = listener.local_addr().unwrap().port();
    let f = Fixture::new();
    let tusd = start_tusd(&f.uploads_dir(), gateway_port, &mut names).await;
    let home = home_at(f, Some(format!("127.0.0.1:{}", tusd.port))).await;
    let (entry, phone) = connect_phone(&home, listener).await;
    Setup {
        home,
        phone,
        tusd,
        _names: names,
        _entry: entry,
    }
}

/// 起 tusd 并等它开始监听。
///
/// 端口由 tusd 自己挑（`-port=0`）：`free_port` 那种先绑住、拿到号码就放掉的做法，到 tusd 真去绑之间隔着建 home
/// 和 `docker run` 的几百毫秒，宿主上别的进程（出站连接也从这段临时端口里取）能把它抢走，tusd 打不开监听就退出。
async fn start_tusd(uploads: &Path, gateway: u16, names: &mut Names) -> Tusd {
    let name = names.oneshot_container("tusd");
    let mut tusd = Tusd::start(uploads, gateway, name);
    assert!(
        tusd.wait_ready().await,
        "tusd 没打开监听：{}",
        tusd.logs().unwrap_or_default()
    );
    tusd
}

/// 在这个 home 上接一部手机：服务网关、开隧道入口、连隧道。
async fn connect_phone(home: &Home, listener: TcpListener) -> (TunnelEntry, Phone) {
    let app = home.app.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let key = SecretKey::generate();
    home.tunnel.devices.add(key.public(), None).unwrap();
    let entry = TunnelEntry::spawn(&home.tunnel, home.app.clone());
    let phone = Phone::connect(key, &home.tunnel).await;
    (entry, phone)
}

/// tusd 就绪那行里报的端口。
///
/// ```text
/// 2026/11/07 00:35:59.167452 You can now upload files to: http://127.0.0.1:46631/_wonton/files/uploads/
/// ```
fn listening_port(logs: &str) -> Option<u16> {
    logs.lines().rev().find_map(|line| {
        let (_, port) = line.split_once("You can now upload files to: http://127.0.0.1:")?;
        port.split('/').next()?.parse().ok()
    })
}

struct Answer {
    status: StatusCode,
    headers: hyper::HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    async fn read(response: Response<hyper::body::Incoming>) -> Self {
        let (parts, body) = response.into_parts();
        Self {
            status: parts.status,
            headers: parts.headers,
            body: body.collect().await.unwrap().to_bytes().to_vec(),
        }
    }

    fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", |v| v.to_str().unwrap())
    }

    fn offset(&self) -> u64 {
        self.header("upload-offset").parse().unwrap()
    }
}

impl Setup {
    fn tus(&self, method: Method, path: &str) -> hyper::http::request::Builder {
        self.phone
            .request(method, CORE, path)
            .header("x-wonton-files", "1")
            .header("tus-resumable", "1.0.0")
    }

    async fn send(&self, request: hyper::Request<Body>) -> Answer {
        let response = self.phone.http().send(request).await.unwrap();
        Answer::read(response).await
    }

    async fn call(&self, method: Method, path: &str) -> Answer {
        self.send(self.tus(method, path).body(Body::empty()).unwrap())
            .await
    }

    /// 创建上传，返回编号。文件名 `a.bin`（base64 是 `YS5iaW4=`）。
    async fn create(&self, size: u64) -> FileId {
        let request = self
            .tus(Method::POST, "/_wonton/files/uploads/")
            .header("upload-length", size.to_string())
            .header("upload-metadata", "filename YS5iaW4=")
            .body(Body::empty())
            .unwrap();
        let created = self.send(request).await;
        assert_eq!(
            created.status,
            StatusCode::CREATED,
            "the core answered {}: {}\nx-wonton-error: {} x-wonton-tunnel-error: {}\n{}",
            created.status,
            String::from_utf8_lossy(&created.body),
            created.header("x-wonton-error"),
            created.header("x-wonton-tunnel-error"),
            self.tusd.logs().unwrap_or_default()
        );
        let location = created.header("location");
        let id = location
            .rsplit_once("/_wonton/files/uploads/")
            .expect("Location 对得上网关的路径")
            .1;
        assert!(
            location.contains(CORE),
            "Location 用的是网关的地址：{location}"
        );
        id.parse().expect("编号是核心发的")
    }

    /// 上一个 PATCH 还没放开 tusd 的锁时 HEAD 要等。
    async fn offset(&self, path: &str) -> u64 {
        for _ in 0..200 {
            let answer = self.call(Method::HEAD, path).await;
            if answer.status == StatusCode::OK {
                return answer.offset();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("问不到进度：{}", self.tusd.logs().unwrap_or_default());
    }
}

/// 请求体由 `chunks` 喂进来的 PATCH。
fn patch_body(mut chunks: mpsc::Receiver<Bytes>) -> Body {
    Body::from_stream(stream::poll_fn(move |cx| {
        chunks
            .poll_recv(cx)
            .map(|chunk| chunk.map(Ok::<_, Infallible>))
    }))
}

async fn wait_for_size(path: &Path, at_least: u64) {
    for _ in 0..200 {
        if std::fs::metadata(path).is_ok_and(|meta| meta.len() >= at_least) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("等了 10 秒 {} 还没到 {at_least} 字节", path.display());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用它起一次性的 tusd"]
async fn uploads_through_the_tunnel_stream_resume_and_end_as_a_file() {
    let s = setup().await;
    let data: Vec<u8> = (0..8 * (1 << 20)).map(|i: u32| (i % 251) as u8).collect();
    let size = data.len() as u64;
    let id = s.create(size).await;
    let path = format!("/_wonton/files/uploads/{id}");
    let stored = s.home.f.uploads_dir().join(id.as_str());
    assert_eq!(s.offset(&path).await, 0);

    // 只送一部分，请求还开着时 tusd 的文件就在长：字节是流式转发的，不是攒完才发。
    let (feed, chunks) = mpsc::channel(4);
    let request = s
        .tus(Method::PATCH, &path)
        .header("upload-offset", "0")
        .header("content-type", "application/offset+octet-stream")
        .body(patch_body(chunks))
        .unwrap();
    let http = s.phone.http();
    let patch = tokio::spawn(async move { http.send(request).await });
    for chunk in data[..4 * CHUNK].chunks(CHUNK) {
        feed.send(Bytes::copy_from_slice(chunk)).await.unwrap();
    }
    wait_for_size(&stored, CHUNK as u64).await;
    assert!(!patch.is_finished(), "请求还开着");

    // 断开，再问进度，从那里接着传。
    patch.abort();
    drop(feed);
    let offset = s.offset(&path).await;
    assert!(offset >= CHUNK as u64, "断开前的字节留着：{offset}");
    assert!(offset <= size);

    let rest = s
        .tus(Method::PATCH, &path)
        .header("upload-offset", offset.to_string())
        .header("content-type", "application/offset+octet-stream")
        .body(Body::from(
            data[usize::try_from(offset).unwrap()..].to_vec(),
        ))
        .unwrap();
    let finished = s.send(rest).await;
    assert_eq!(
        finished.status,
        StatusCode::NO_CONTENT,
        "{}",
        s.tusd.logs().unwrap_or_default()
    );
    assert_eq!(finished.offset(), size);

    // 最后一个 PATCH 回来时文件已经可用，不用再等。
    let info = s.call(Method::GET, &format!("/_wonton/files/{id}")).await;
    assert_eq!(info.status, StatusCode::OK);
    let info: serde_json::Value = serde_json::from_slice(&info.body).unwrap();
    assert_eq!(info["name"], "a.bin");
    assert_eq!(info["size"], size);
    assert_eq!(info["source"]["type"], "device");
    let content = s
        .call(Method::GET, &format!("/_wonton/files/{id}/content"))
        .await;
    assert_eq!(content.body, data);
    let placed = s.home.f.shares_dir.join("transfers/a.bin");
    assert_eq!(std::fs::metadata(&placed).unwrap().len(), size);
    assert!(!stored.exists());

    let head = s.call(Method::HEAD, &path).await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(head.offset(), size);

    // 重名的加序号。
    let second = s.create(1).await;
    let request = s
        .tus(Method::PATCH, &format!("/_wonton/files/uploads/{second}"))
        .header("upload-offset", "0")
        .header("content-type", "application/offset+octet-stream")
        .body(Body::from("x"))
        .unwrap();
    assert_eq!(s.send(request).await.status, StatusCode::NO_CONTENT);
    assert!(s.home.f.shares_dir.join("transfers/a (1).bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用它起一次性的 tusd"]
async fn tusd_passes_the_cores_refusals_through_and_cancels() {
    let s = setup().await;
    // 放不下：核心的钩子说的状态码、错误体和响应头原样到手机。
    let request = s
        .tus(Method::POST, "/_wonton/files/uploads/")
        .header("upload-length", (u64::MAX / 2).to_string())
        .header("upload-metadata", "filename YS5iaW4=")
        .body(Body::empty())
        .unwrap();
    let full = s.send(request).await;
    assert_eq!(
        full.status,
        StatusCode::INSUFFICIENT_STORAGE,
        "{}",
        s.tusd.logs().unwrap_or_default()
    );
    assert_eq!(full.header("x-wonton-error"), "storage_full");
    let body: serde_json::Value = serde_json::from_slice(&full.body).unwrap();
    assert_eq!(body["error"]["kind"], "storage_full");

    let request = s
        .tus(Method::POST, "/_wonton/files/uploads/")
        .header("upload-defer-length", "1")
        .header("upload-metadata", "filename YS5iaW4=")
        .body(Body::empty())
        .unwrap();
    let deferred = s.send(request).await;
    assert_eq!(deferred.status, StatusCode::BAD_REQUEST);
    assert_eq!(deferred.header("x-wonton-error"), "invalid_input");

    // 空文件没有 PATCH，创建完就是文件。
    let empty = s.create(0).await;
    let info = s
        .call(Method::GET, &format!("/_wonton/files/{empty}"))
        .await;
    assert_eq!(
        info.status,
        StatusCode::OK,
        "{}",
        s.tusd.logs().unwrap_or_default()
    );

    // 取消：数据删掉，编号不再认。
    let id = s.create(10).await;
    let path = format!("/_wonton/files/uploads/{id}");
    let stored = s.home.f.uploads_dir().join(id.as_str());
    assert!(stored.exists());
    assert_eq!(
        s.call(Method::DELETE, &path).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(!stored.exists());
    let gone = s.call(Method::HEAD, &path).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.header("x-wonton-error"), "file_unknown");
}

/// 端口上没人监听时如实说连不上，不和“上游出问题”混成一个类别。
#[tokio::test(flavor = "multi_thread")]
async fn a_tusd_that_is_not_running_shows_as_unavailable() {
    let port = free_port();
    let home = home_with(Some(format!("127.0.0.1:{port}"))).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_entry, phone) = connect_phone(&home, listener).await;
    let request = phone
        .request(Method::POST, CORE, "/_wonton/files/uploads/")
        .header("x-wonton-files", "1")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "10")
        .header("upload-metadata", "filename YS5iaW4=")
        .body(Body::empty())
        .unwrap();
    let answer = Answer::read(phone.http().send(request).await.unwrap()).await;
    assert_eq!(
        answer.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    assert_eq!(answer.header("x-wonton-error"), "service_unavailable");
}

/// 端口已经被别人占着时照样能用：`free_port` 拿到的号码交出去之前就可能被抢走。
///
/// 先后顺序是钉住的：抢端口的监听者先起，而且收下连接就断（连接立刻被拒）；之后才起 tusd。
/// 修好以前 tusd 绑不上这个端口就退出，测试却连上了抢端口的东西，创建上传拿到 503（对方收下连接就断）。
/// 抢端口的自己也绑 0：先挑号再绑，中间会被并行的测试抢走（CI 上见过 `AddrInUse`）。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用它起一次性的 tusd"]
async fn an_upload_works_when_the_port_is_already_taken() {
    let thief = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let stolen = thief.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((socket, _)) = thief.accept().await {
            drop(socket);
        }
    });
    let s = setup().await;
    assert_ne!(s.tusd.port, stolen, "tusd 用的是自己挑的端口");
    let id = s.create(1).await;
    assert_eq!(
        s.call(Method::HEAD, &format!("/_wonton/files/uploads/{id}"))
            .await
            .offset(),
        0
    );
}
