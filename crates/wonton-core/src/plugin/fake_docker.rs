//! 测试用的假 Docker Engine API：只实现插件安装、核对、卸载和回滚用到的端点，记下现存的容器、网络和卷
//! （连同标签），供测试断言对 Docker 的副作用。容器一经创建默认报告为运行中且健康，
//! 测试可以按容器名改成别的状态（已退出、没有健康检查等）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore};

use super::docker::Engine;
use super::names::LABEL_PLUGIN;

#[derive(Clone, Default)]
pub struct FakeDocker {
    resources: Arc<Mutex<Resources>>,
    /// [`Self::pause`] 拿着写锁时请求都等着：每个请求先拿读锁再处理。
    gate: Arc<tokio::sync::RwLock<()>>,
}

/// 容器里执行的命令怎么回应：（容器名，argv，标准输入）→（退出码，标准错误）。
pub type ExecHandler = Arc<dyn Fn(&str, &[String], &[u8]) -> (i64, String) + Send + Sync>;

/// 一次 `docker exec`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeExec {
    pub container: String,
    pub cmd: Vec<String>,
    pub stdin: Vec<u8>,
    exit_code: Option<i64>,
}

/// 容器里的一个路径。
#[derive(Clone)]
pub enum FakeFile {
    Regular(Vec<u8>),
    Directory,
}

/// 停住的请求，见 [`FakeDocker::hold`]。
#[derive(Clone)]
pub struct Hold {
    call: String,
    reached: Arc<Notify>,
    released: Arc<Semaphore>,
}

impl Hold {
    /// 等到假 Docker 收到这个请求。
    pub async fn reached(&self) {
        self.reached.notified().await;
    }

    /// 放行停住的请求，之后的同样请求也不再停。
    pub fn release(&self) {
        self.released.close();
    }
}

/// 资源的标签。
type Labels = BTreeMap<String, String>;

#[derive(Default)]
struct Resources {
    /// 容器名 → 标签。
    containers: BTreeMap<String, Labels>,
    /// 容器名 → 创建它的请求。
    created: BTreeMap<String, Value>,
    /// 网络名 → 标签。
    networks: BTreeMap<String, Labels>,
    /// 网络名 → 创建时的请求（`Internal`、`Options` 等）。
    network_bodies: BTreeMap<String, Value>,
    /// 卷名 → 标签。
    volumes: BTreeMap<String, Labels>,
    /// 收到的写操作，按顺序，如 `POST /containers/x/start`。
    writes: Vec<String>,
    /// 写进容器的归档：容器名 → 归档内容。
    archives: BTreeMap<String, Vec<u8>>,
    /// 容器里能经归档接口读到的路径：（容器名，路径）→ 内容。
    files: BTreeMap<(String, String), FakeFile>,
    /// 按容器名指定的状态（Docker inspect 里的 `State`），没指定的报告为运行中且健康。
    states: BTreeMap<String, Value>,
    /// 网络名 → 最近一次把容器接入它的请求（`EndpointConfig.Aliases` 等）。
    connections: BTreeMap<String, Value>,
    /// 接入（`connect`）后还没退出（`disconnect`）的（网络名，容器名）。
    connected: BTreeSet<(String, String)>,
    /// exec ID → 这次执行，按创建顺序编号。
    execs: BTreeMap<String, FakeExec>,
    /// 没设时命令都以 0 退出。
    exec_handler: Option<ExecHandler>,
    /// 设了时本地没有镜像，拉取时 Docker 回这个错误；没设时镜像都在本地。
    pull_error: Option<String>,
    /// Docker 登记的运行时；没设时只有 runc。
    runtimes: Option<Vec<String>>,
    /// 镜像 → 它自带的 `Healthcheck`（Docker 的 JSON 写法），没设的没有。
    image_healthchecks: BTreeMap<String, Value>,
    /// 容器名 → 跑完时的（退出码，标准输出，标准错误），`wait` 和 `logs` 按它回答；没设的以 0 退出、没有输出。
    outputs: BTreeMap<String, (i64, String, String)>,
    hold: Option<Hold>,
}

impl FakeDocker {
    /// 启动假 Docker，返回它和连到它的 [`Engine`]。
    pub async fn start() -> (Self, Engine) {
        let fake = Self::default();
        let app = Router::new().fallback(handle).with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (fake, Engine::http_for_test(&format!("http://{addr}")))
    }

    /// 放进一个运行中的容器，比如核心重启前就装好的插件的容器。
    pub fn add_container(&self, name: &str, plugin: &str) {
        self.lock()
            .containers
            .insert(name.to_owned(), owned_by(plugin));
    }

    pub fn containers(&self) -> Vec<String> {
        self.lock().containers.keys().cloned().collect()
    }

    pub fn networks(&self) -> Vec<String> {
        self.lock().networks.keys().cloned().collect()
    }

    pub fn volumes(&self) -> Vec<String> {
        self.lock().volumes.keys().cloned().collect()
    }

    /// 放进一个已有的网络，比如核心重启前就装好的插件的网络；`body` 是创建它的请求。
    pub fn add_network(&self, name: &str, plugin: &str, body: Value) {
        let mut docker = self.lock();
        docker.networks.insert(name.to_owned(), owned_by(plugin));
        docker.network_bodies.insert(name.to_owned(), body);
    }

    /// 放进一个没有标签的网络，比如主人自己建的。
    pub fn add_unlabeled_network(&self, name: &str) {
        let mut docker = self.lock();
        docker.networks.insert(name.to_owned(), Labels::new());
        docker.network_bodies.insert(name.to_owned(), json!({}));
    }

    /// 放进一个已有的卷，比如别的插件的。
    pub fn add_volume(&self, name: &str, plugin: &str) {
        self.lock()
            .volumes
            .insert(name.to_owned(), owned_by(plugin));
    }

    /// 放进一个不是核心创建的容器，`body` 是创建它的请求。
    pub fn add_stranger_container(&self, name: &str, body: Value) {
        let mut docker = self.lock();
        docker.containers.insert(name.to_owned(), Labels::new());
        docker.created.insert(name.to_owned(), body);
    }

    /// 创建网络时的请求。
    pub fn network(&self, name: &str) -> Option<Value> {
        self.lock().network_bodies.get(name).cloned()
    }

    pub fn writes(&self) -> Vec<String> {
        self.lock().writes.clone()
    }

    /// 最近一次接入这个网络的容器在网络里的别名。
    pub fn aliases(&self, network: &str) -> Vec<String> {
        let connection = self.lock().connections.get(network).cloned();
        let aliases = connection.map(|c| c["EndpointConfig"]["Aliases"].clone());
        aliases
            .and_then(|a| serde_json::from_value(a).ok())
            .unwrap_or_default()
    }

    /// 最近一次接入这个网络时要的网关优先级（`EndpointConfig.GwPriority`），没写时为空。
    pub fn gateway_priority(&self, network: &str) -> Option<i64> {
        let connection = self.lock().connections.get(network).cloned();
        connection.and_then(|c| c["EndpointConfig"]["GwPriority"].as_i64())
    }

    /// 收到这个请求（如 `DELETE /containers/x`）时先停住，等 [`Hold::release`] 再处理。
    pub fn hold(&self, call: &str) -> Hold {
        let hold = Hold {
            call: call.to_owned(),
            reached: Arc::new(Notify::new()),
            released: Arc::new(Semaphore::new(0)),
        };
        self.lock().hold = Some(hold.clone());
        hold
    }

    /// 最近一次写进这个容器的 tar 归档。
    pub fn archive(&self, container: &str) -> Option<Vec<u8>> {
        self.lock().archives.get(container).cloned()
    }

    /// 容器最近一次创建时的环境变量。
    pub fn env(&self, container: &str) -> Vec<String> {
        let created = self.created(container).unwrap_or_default();
        let env = created["Env"].as_array().cloned().unwrap_or_default();
        env.iter()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect()
    }

    /// 容器最近一次创建时的请求。
    pub fn created(&self, container: &str) -> Option<Value> {
        self.lock().created.get(container).cloned()
    }

    /// 让容器里出现（`Some`）或去掉（`None`）一个路径，比如插件运行中写下的文件。
    pub fn set_file(&self, container: &str, path: &str, file: Option<FakeFile>) {
        let key = (container.to_owned(), path.to_owned());
        let mut docker = self.lock();
        match file {
            Some(file) => docker.files.insert(key, file),
            None => docker.files.remove(&key),
        };
    }

    /// 让这个名字的容器（现在的或之后创建的）报告给定的状态，如
    /// `{"Status": "exited", "ExitCode": 0}`。
    pub fn set_state(&self, container: &str, state: Value) {
        self.lock().states.insert(container.to_owned(), state);
    }

    /// 容器里执行的命令由 `handler` 回应。
    pub fn on_exec(
        &self,
        handler: impl Fn(&str, &[String], &[u8]) -> (i64, String) + Send + Sync + 'static,
    ) {
        self.lock().exec_handler = Some(Arc::new(handler));
    }

    /// 之后镜像都不在本地、拉取时 Docker 回 `message`（500）；`None` 恢复成镜像都在本地。
    pub fn fail_pulls(&self, message: Option<&str>) {
        self.lock().pull_error = message.map(str::to_owned);
    }

    /// 暂停回应（请求照收），直到丢掉返回的守卫，比如让卸载停在半路。
    pub async fn pause(&self) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.gate.clone().write_owned().await
    }

    /// 执行过的命令，按顺序。
    pub fn execs(&self) -> Vec<FakeExec> {
        self.lock().execs.values().cloned().collect()
    }

    /// 放进一个从 `image` 创建的运行中的容器，比如核心自己的容器。
    pub fn add_image_container(&self, name: &str, image: &str) {
        let mut docker = self.lock();
        docker.containers.insert(name.to_owned(), Labels::new());
        docker
            .created
            .insert(name.to_owned(), json!({"Image": image}));
    }

    /// 这个名字的容器（之后创建的）跑完时的退出码和输出。
    pub fn set_output(&self, container: &str, code: i64, stdout: &str, stderr: &str) {
        let output = (code, stdout.to_owned(), stderr.to_owned());
        self.lock().outputs.insert(container.to_owned(), output);
    }

    /// 让 `GET /info` 报告这些运行时（如 `runc`、`nvidia`）。
    /// 给镜像设上它自带的 `HEALTHCHECK`（Docker 的 JSON 写法）。
    pub fn set_image_healthcheck(&self, image: &str, check: Value) {
        self.lock()
            .image_healthchecks
            .insert(image.to_owned(), check);
    }

    pub fn set_runtimes(&self, runtimes: &[&str]) {
        self.lock().runtimes = Some(runtimes.iter().map(|r| (*r).to_owned()).collect());
    }

    fn lock(&self) -> MutexGuard<'_, Resources> {
        self.resources.lock().expect("假 Docker 的锁中毒")
    }
}

/// `POST /exec/<id>/start`：和 Docker 一样升级成原始连接，读完标准输入（客户端关闭写端）后按 Docker 的多路
/// 复用格式写回标准错误，再关闭连接。
fn start_exec(fake: FakeDocker, id: &str, mut request: Request) -> Response {
    let Some(exec) = fake.lock().execs.get(id).cloned() else {
        return gone(false);
    };
    let upgrade = hyper::upgrade::on(&mut request);
    let id = id.to_owned();
    tokio::spawn(async move {
        let mut io = TokioIo::new(upgrade.await.expect("exec 连接升级"));
        let mut stdin = Vec::new();
        io.read_to_end(&mut stdin)
            .await
            .expect("读 exec 的标准输入");
        let handler = fake.lock().exec_handler.clone();
        let (code, stderr) = match handler {
            Some(handler) => handler(&exec.container, &exec.cmd, &stdin),
            None => (0, String::new()),
        };
        if let Some(exec) = fake.lock().execs.get_mut(&id) {
            exec.stdin = stdin;
            exec.exit_code = Some(code);
        }
        if !stderr.is_empty() {
            let mut frame = vec![2, 0, 0, 0];
            frame.extend_from_slice(&u32::try_from(stderr.len()).unwrap().to_be_bytes());
            frame.extend_from_slice(stderr.as_bytes());
            io.write_all(&frame).await.expect("写 exec 的标准错误");
        }
        io.shutdown().await.ok();
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "tcp")
        .body(Body::empty())
        .unwrap()
}

async fn handle(State(fake): State<FakeDocker>, request: Request) -> Response {
    let _resumed = fake.gate.clone().read_owned().await;
    let (method, uri) = (request.method().clone(), request.uri().clone());
    let exec_start = uri
        .path()
        .split('/')
        .collect::<Vec<_>>()
        .windows(3)
        .find(|w| w[0] == "exec" && w[2] == "start")
        .map(|w| w[1].to_owned());
    if let Some(id) = exec_start {
        fake.lock()
            .writes
            .push(format!("{method} /exec/{id}/start"));
        return start_exec(fake, &id, request);
    }
    let hold = fake.lock().hold.clone();
    if let Some(hold) = hold.filter(|h| h.call == format!("{method} {}", api_path(&uri))) {
        hold.reached.notify_one();
        // 放行时信号量关闭，acquire 返回错误。
        hold.released.acquire().await.ok();
    }
    let body = request
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    serve(&fake, &method, &uri, &body)
}

/// 去掉 bollard 的请求路径可能带的 API 版本前缀（/v1.51/...）。
fn api_path(uri: &Uri) -> &str {
    let path = uri.path();
    match path.strip_prefix("/v") {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => {
            rest.find('/').map_or("/", |i| &rest[i..])
        }
        _ => path,
    }
}

fn serve(fake: &FakeDocker, method: &Method, uri: &Uri, body: &Bytes) -> Response {
    let path = api_path(uri);
    let call = format!("{method} {path}");
    let query = uri.query().unwrap_or_default();
    let mut docker = fake.lock();
    if *method != Method::GET {
        docker.writes.push(call.clone());
    }
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let reply = match parts.as_slice() {
        ["containers", rest @ ..] => containers(&mut docker, method, rest, query, body),
        ["networks", rest @ ..] => networks(&mut docker, method, rest, query, body),
        _ => others(&mut docker, method, &parts, query, body),
    };
    reply.unwrap_or_else(|| {
        json_reply(
            StatusCode::NOT_FOUND,
            json!({"message": format!("假 Docker 没有 {call}")}),
        )
    })
}

/// `/containers/...` 的端点。
fn containers(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    let exists = |name: &str| docker.containers.contains_key(name);
    let reply = match (method, parts) {
        (&Method::POST, ["create"])
            if query_param(query, "name").is_some_and(|name| exists(&name)) =>
        {
            let name = query_param(query, "name").unwrap_or_default();
            let message = format!("Conflict. The container name \"/{name}\" is already in use");
            json_reply(StatusCode::CONFLICT, json!({"message": message}))
        }
        (&Method::POST, ["create"]) => {
            let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
            let name = query_param(query, "name").unwrap_or_default();
            docker.containers.insert(name.clone(), labels_of(&body));
            docker.created.insert(name.clone(), body);
            json_reply(StatusCode::CREATED, json!({"Id": name, "Warnings": []}))
        }
        (&Method::POST, [name, "start" | "stop"]) => gone(exists(name)),
        (&Method::DELETE, [name]) => gone(docker.containers.remove(*name).is_some()),
        (&Method::GET, [name, "json"]) if exists(name) => {
            let healthy =
                json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}});
            let state = docker.states.get(*name).cloned().unwrap_or(healthy);
            // 容器在它加入的每个网络里的地址都报成 127.0.0.1：测试在本机起假的插件网页。
            let created = docker.created.get(*name).cloned().unwrap_or_default();
            let endpoints = created["NetworkingConfig"]["EndpointsConfig"]
                .as_object()
                .cloned();
            let networks: serde_json::Map<String, Value> = endpoints
                .unwrap_or_default()
                .keys()
                .map(|network| (network.clone(), json!({"IPAddress": "127.0.0.1"})))
                .collect();
            json_reply(
                StatusCode::OK,
                json!({"Name": format!("/{name}"), "State": state, "Image": created["Image"],
                       "HostConfig": created["HostConfig"],
                       "NetworkSettings": {"Networks": networks}}),
            )
        }
        (&Method::POST, [name, "wait"]) if exists(name) => {
            let code = docker.outputs.get(*name).map_or(0, |o| o.0);
            json_reply(StatusCode::OK, json!({"StatusCode": code}))
        }
        (&Method::GET, [name, "logs"]) if exists(name) => {
            let (_, stdout, stderr) = docker.outputs.get(*name).cloned().unwrap_or_default();
            let mut frames = Vec::new();
            // 和 Docker 一样多路复用：1 是标准输出，2 是标准错误。
            for (stream, text) in [(1u8, stdout), (2, stderr)] {
                if !text.is_empty() {
                    frames.extend_from_slice(&[stream, 0, 0, 0]);
                    frames.extend_from_slice(&u32::try_from(text.len()).unwrap().to_be_bytes());
                    frames.extend_from_slice(text.as_bytes());
                }
            }
            (StatusCode::OK, frames).into_response()
        }
        (&Method::POST, [name, "exec"]) if exists(name) => {
            let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
            let cmd = serde_json::from_value(body["Cmd"].clone()).unwrap_or_default();
            let id = format!("exec{}", docker.execs.len() + 1);
            let exec = FakeExec {
                container: (*name).to_owned(),
                cmd,
                stdin: Vec::new(),
                exit_code: None,
            };
            docker.execs.insert(id.clone(), exec);
            json_reply(StatusCode::CREATED, json!({"Id": id}))
        }
        (&Method::PUT, [name, "archive"]) if exists(name) => {
            docker.archives.insert((*name).to_owned(), raw.to_vec());
            StatusCode::OK.into_response()
        }
        (&Method::GET, [name, "archive"]) if exists(name) => {
            let path = query_param(query, "path").unwrap_or_default();
            match docker.files.get(&((*name).to_owned(), path.clone())) {
                Some(file) => (StatusCode::OK, archive_of(&path, file)).into_response(),
                None => json_reply(
                    StatusCode::NOT_FOUND,
                    json!({"message": format!("Could not find the file {path} in container {name}")}),
                ),
            }
        }
        (&Method::GET, ["json"]) => {
            let list: Vec<Value> = labeled(&docker.containers, query)
                .map(|(n, labels)| json!({"Names": [format!("/{n}")], "Labels": labels}))
                .collect();
            json_reply(StatusCode::OK, json!(list))
        }
        _ => return None,
    };
    Some(reply)
}

/// `/networks/...` 的端点。
fn networks(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let reply = match (method, parts) {
        (&Method::POST, ["create"])
            if body["Name"]
                .as_str()
                .is_some_and(|name| docker.networks.contains_key(name)) =>
        {
            let name = body["Name"].as_str().unwrap_or_default();
            let message = format!("network with name {name} already exists");
            json_reply(StatusCode::CONFLICT, json!({"message": message}))
        }
        (&Method::POST, ["create"]) => {
            let name = body["Name"].as_str().unwrap_or_default().to_owned();
            docker.networks.insert(name.clone(), labels_of(&body));
            docker.network_bodies.insert(name.clone(), body);
            json_reply(StatusCode::CREATED, json!({"Id": name, "Warning": ""}))
        }
        (&Method::POST, [name, action @ ("connect" | "disconnect")]) => {
            let found = docker.networks.contains_key(*name);
            if found && *action == "connect" {
                let container = body["Container"].as_str().unwrap_or_default().to_owned();
                docker.connected.insert(((*name).to_owned(), container));
                docker.connections.insert((*name).to_owned(), body);
            } else if found {
                let container = body["Container"].as_str().unwrap_or_default().to_owned();
                docker.connected.remove(&((*name).to_owned(), container));
            }
            if found {
                StatusCode::OK.into_response()
            } else {
                gone(false)
            }
        }
        (&Method::GET, [name]) if docker.networks.contains_key(*name) => {
            let created = &docker.network_bodies[*name];
            json_reply(
                StatusCode::OK,
                json!({
                    "Name": name, "Internal": created["Internal"], "Options": created["Options"],
                    "Labels": docker.networks[*name],
                    "Containers": endpoints_of(docker, name).iter()
                        .map(|c| (format!("id-{c}"), json!({"Name": c})))
                        .collect::<serde_json::Map<String, Value>>(),
                    "IPAM": {"Config": [{"Subnet": "172.30.0.0/24", "Gateway": "172.30.0.1"}]}
                }),
            )
        }
        (&Method::DELETE, [name]) if !endpoints_of(docker, name).is_empty() => {
            let message =
                format!("error while removing network: network {name} has active endpoints");
            json_reply(StatusCode::FORBIDDEN, json!({"message": message}))
        }
        (&Method::DELETE, [name]) => {
            docker.network_bodies.remove(*name);
            gone(docker.networks.remove(*name).is_some())
        }
        (&Method::GET, []) => {
            let list: Vec<Value> = labeled(&docker.networks, query)
                .map(|(n, labels)| {
                    json!({"Name": n, "Id": n, "Driver": "bridge", "Labels": labels,
                           "Options": docker.network_bodies[n]["Options"],
                           "IPAM": {"Config": [{"Subnet": "172.30.0.0/24", "Gateway": "172.30.0.1"}]}})
                })
                .collect();
            json_reply(StatusCode::OK, json!(list))
        }
        _ => return None,
    };
    Some(reply)
}

/// 版本、镜像、卷。
fn others(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let reply = match (method, parts) {
        (&Method::GET, ["version"]) => json_reply(
            StatusCode::OK,
            json!({"Arch": "x86_64", "Version": "29.0.0"}),
        ),
        (&Method::GET, ["images", .., "json"]) if docker.pull_error.is_some() => {
            json_reply(StatusCode::NOT_FOUND, json!({"message": "No such image"}))
        }
        (&Method::GET, ["info"]) => {
            let runtimes = docker
                .runtimes
                .clone()
                .unwrap_or_else(|| vec!["runc".to_owned()]);
            let map: serde_json::Map<String, Value> = runtimes
                .into_iter()
                .map(|name| (name.clone(), json!({"path": name})))
                .collect();
            json_reply(StatusCode::OK, json!({"Runtimes": map}))
        }
        (&Method::GET, ["images", image @ .., "json"]) => {
            let check = docker.image_healthchecks.get(&image.join("/"));
            json_reply(StatusCode::OK, json!({"Config": {"Healthcheck": check}}))
        }
        (&Method::POST, ["images", "create"]) if docker.pull_error.is_some() => json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": docker.pull_error}),
        ),
        (&Method::GET, ["exec", id, "json"]) if docker.execs.contains_key(*id) => {
            let code = docker.execs[*id].exit_code;
            json_reply(
                StatusCode::OK,
                json!({"ExitCode": code, "Running": code.is_none()}),
            )
        }
        (&Method::POST, ["volumes", "create"]) => {
            let name = body["Name"].as_str().unwrap_or_default().to_owned();
            let labels = labels_of(&body);
            let reply = volume(&name, &labels);
            docker.volumes.insert(name, labels);
            json_reply(StatusCode::CREATED, reply)
        }
        (&Method::GET, ["volumes", name]) if docker.volumes.contains_key(*name) => {
            json_reply(StatusCode::OK, volume(name, &docker.volumes[*name]))
        }
        (&Method::DELETE, ["volumes", name]) => gone(docker.volumes.remove(*name).is_some()),
        (&Method::GET, ["volumes"]) => {
            let list: Vec<Value> = labeled(&docker.volumes, query)
                .map(|(n, labels)| volume(n, labels))
                .collect();
            json_reply(StatusCode::OK, json!({"Volumes": list, "Warnings": []}))
        }
        _ => return None,
    };
    Some(reply)
}

/// 接在网络上的容器：创建时接进来还没删的，和接入（`connect`）后没退出的。
fn endpoints_of(docker: &Resources, network: &str) -> BTreeSet<String> {
    let joined = |name: &&String| {
        let created = docker.created.get(*name);
        created.is_some_and(|c| !c["NetworkingConfig"]["EndpointsConfig"][network].is_null())
    };
    let created = docker.containers.keys().filter(joined).cloned();
    let connected = docker.connected.iter().filter(|(n, _)| n == network);
    created
        .chain(connected.map(|(_, container)| container.clone()))
        .collect()
}

/// 和 Docker 一样：归档里只有这一项，名字是路径的最后一级。
fn archive_of(path: &str, file: &FakeFile) -> Vec<u8> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    let data: &[u8] = match file {
        FakeFile::Regular(data) => {
            header.set_entry_type(tar::EntryType::Regular);
            data
        }
        FakeFile::Directory => {
            header.set_entry_type(tar::EntryType::Directory);
            &[]
        }
    };
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    builder.append_data(&mut header, name, data).unwrap();
    builder.into_inner().unwrap()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, value)| value.into_owned())
}

fn json_reply(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// 删除或启停：资源存在时 204，否则 404（和 Docker 一样）。
fn gone(found: bool) -> Response {
    if found {
        StatusCode::NO_CONTENT.into_response()
    } else {
        json_reply(
            StatusCode::NOT_FOUND,
            json!({"message": "No such resource"}),
        )
    }
}

fn owned_by(plugin: &str) -> Labels {
    Labels::from([(LABEL_PLUGIN.to_owned(), plugin.to_owned())])
}

fn labels_of(body: &Value) -> Labels {
    serde_json::from_value(body["Labels"].clone()).unwrap_or_default()
}

fn volume(name: &str, labels: &Labels) -> Value {
    json!({
        "Name": name, "Driver": "local", "Mountpoint": format!("/var/lib/docker/volumes/{name}"),
        "Labels": labels, "Options": {}, "Scope": "local"
    })
}

/// 按查询里的 `filters={"label":["键=值", "键"]}` 筛出资源：每一条都要满足，和 Docker 一样。
fn labeled<'a>(
    resources: &'a BTreeMap<String, Labels>,
    query: &str,
) -> impl Iterator<Item = (&'a String, &'a Labels)> {
    let wanted: Vec<String> = query_param(query, "filters")
        .and_then(|filters| serde_json::from_str::<Value>(&filters).ok())
        .and_then(|filters| serde_json::from_value(filters["label"].clone()).ok())
        .unwrap_or_default();
    resources.iter().filter(move |(_, labels)| {
        wanted.iter().all(|filter| match filter.split_once('=') {
            Some((key, value)) => labels.get(key).map(String::as_str) == Some(value),
            None => labels.contains_key(filter),
        })
    })
}
