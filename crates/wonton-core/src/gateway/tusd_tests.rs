//! 真的 tusd、真的隧道：确认 ADR-0020 里 tusd 接得上的几点。tusd 用部署时的镜像和参数，开在主机网络上（只有 Linux 上
//! 才连得回测试进程），所以只在 Linux 上编译。

#![cfg(target_os = "linux")]

use std::convert::Infallible;
use std::net::SocketAddr;
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

use super::files_tests::{Home, home_with};
use super::files_upload_tests::deployed_tusd;
use super::tests::CORE;
use super::tunnel_tests::Phone;
use crate::config::FILE_HOOKS_HOST;
use crate::tunnel::TunnelEntry;

const CHUNK: usize = 256 << 10;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// 退出时删掉的 tusd 容器。
struct Tusd {
    name: String,
    port: u16,
}

impl Tusd {
    /// 镜像和参数取自部署文件，端口换掉。
    fn start(uploads: &Path, gateway: u16, port: u16) -> Self {
        let service = deployed_tusd(crate::install::COMPOSE);
        let name = format!("wonton-tusdtest{}-{port}", std::process::id());
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
            .args(["-host=127.0.0.1".to_owned(), format!("-port={port}")]);
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
        Self { name, port }
    }

    async fn wait_ready(&self) {
        for _ in 0..300 {
            let address = SocketAddr::from(([127, 0, 0, 1], self.port));
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("等了 30 秒 tusd 还没起来：{}", self.logs());
    }

    fn logs(&self) -> String {
        let output = Command::new("docker").args(["logs", &self.name]).output();
        output.map_or_else(
            |e| e.to_string(),
            |o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            },
        )
    }
}

impl Drop for Tusd {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .output();
    }
}

struct Setup {
    home: Home,
    phone: Phone,
    tusd: Tusd,
    _entry: TunnelEntry,
}

async fn setup() -> Setup {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_port = listener.local_addr().unwrap().port();
    let tusd_port = free_port();
    let home = home_with(Some(format!("127.0.0.1:{tusd_port}"))).await;
    let tusd = Tusd::start(&home.f.uploads_dir(), gateway_port, tusd_port);
    tusd.wait_ready().await;
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
    Setup {
        home,
        phone,
        tusd,
        _entry: entry,
    }
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
        assert_eq!(created.status, StatusCode::CREATED, "{}", self.tusd.logs());
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
        panic!("问不到进度：{}", self.tusd.logs());
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
    assert_eq!(finished.status, StatusCode::NO_CONTENT, "{}", s.tusd.logs());
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
        s.tusd.logs()
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
    assert_eq!(info.status, StatusCode::OK, "{}", s.tusd.logs());

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
