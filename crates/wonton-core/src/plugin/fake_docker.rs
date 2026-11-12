//! 测试用的假 Docker Engine API：只实现插件安装、核对、卸载和回滚用到的端点，记下现存的容器、网络和卷
//! （连同标签），供测试断言对 Docker 的副作用。容器一经创建默认报告为运行中且健康，
//! 测试可以按容器名改成别的状态（已退出、没有健康检查等）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, broadcast};

use self::containers::containers;
use self::events::events_response;
use self::exec::start_exec;
use self::networks::networks;
use self::volumes::volumes;
use super::docker::Engine;
use super::names::LABEL_PLUGIN;

mod containers;
mod events;
mod exec;
mod networks;
mod update;
mod volumes;

#[derive(Clone)]
pub struct FakeDocker {
    resources: Arc<Mutex<Resources>>,
    /// [`Self::pause`] 拿着写锁时请求都等着：每个请求先拿读锁再处理。
    gate: Arc<tokio::sync::RwLock<()>>,
    events: broadcast::Sender<Value>,
}

impl Default for FakeDocker {
    fn default() -> Self {
        let (events, _) = broadcast::channel(16);
        Self {
            resources: Arc::default(),
            gate: Arc::default(),
            events,
        }
    }
}

/// 容器里执行的命令怎么回应：（容器名，argv，标准输入）→（退出码，标准错误）。
pub type ExecHandler = Arc<dyn Fn(&str, &[String], &[u8]) -> (i64, String) + Send + Sync>;
/// 可分别回应标准输出和标准错误的 exec 替身。
pub type ExecOutputHandler =
    Arc<dyn Fn(&str, &[String], &[u8]) -> (i64, String, String) + Send + Sync>;

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
    /// 容器名 → 被创建过的次数：重建会计数，断言“容器（别的 Bot 的）没被动过”用。
    creation_count: BTreeMap<String, u32>,
    /// 网络名 → 标签。
    networks: BTreeMap<String, Labels>,
    /// 网络名 → 创建时的请求（`Internal`、`Options` 等）。
    network_bodies: BTreeMap<String, Value>,
    /// 接下来这么多次指定网段的建网络，Docker 回这个错误。
    pinned_network_rejection: Option<(u32, String)>,
    network_collisions: u32,
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
    start_states: BTreeMap<String, Value>,
    start_error: Option<String>,
    /// 网络名 → 最近一次把容器接入它的请求（`EndpointConfig.Aliases` 等）。
    connections: BTreeMap<String, Value>,
    /// 网络名 → 容器名 → 它在网络里的端点（Docker 的 `EndpointSettings` 写法）。
    endpoints: BTreeMap<String, BTreeMap<String, Value>>,
    /// exec ID → 这次执行，按创建顺序编号。
    execs: BTreeMap<String, FakeExec>,
    /// 没设时命令都以 0 退出。
    exec_handler: Option<ExecHandler>,
    exec_output_handler: Option<ExecOutputHandler>,
    /// 设了时本地没有镜像，拉取时 Docker 回这个错误；没设时镜像都在本地。
    pull_error: Option<String>,
    /// 名字里有这段的镜像不受 `pull_error` 影响，还在本地。
    pull_spared: Option<String>,
    /// 查不到的镜像：查它和用它创建容器都回 404。模拟 containerd 镜像存储下镜像记录被清掉（#666）。
    missing_images: BTreeSet<String>,
    /// Docker 登记的运行时；没设时只有 runc。
    runtimes: Option<Vec<String>>,
    /// Docker `/info` 报告的名字（宿主主机名）；没设时不报。
    name: Option<String>,
    /// 设了时 `GET /info` 回这个错误；`None` 恢复正常。
    info_error: Option<String>,
    https_proxy: Option<String>,
    /// 镜像 → 它自带的 `Healthcheck`（Docker 的 JSON 写法），没设的没有。
    image_healthchecks: BTreeMap<String, Value>,
    /// 容器名 → 跑完时的（退出码，标准输出，标准错误），`wait` 和 `logs` 按它回答；没设的以 0 退出、没有输出。
    outputs: BTreeMap<String, (i64, String, String)>,
    /// 下次容器更新时返回 Docker 错误；`None` 恢复正常。
    update_error: Option<String>,
    hold: Option<Hold>,
}

impl FakeDocker {
    /// 连到这个假 Docker 的新 [`Engine`]：进程内的锁是全新的，测试用它当第二个独立核心。
    /// 假状态是共享的，多个 Engine 看到同一份资源。
    pub async fn engine(&self) -> Engine {
        let app = Router::new().fallback(handle).with_state(self.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let engine = Engine::http_for_test(&format!("http://{addr}"));
        let detector = super::lan::Detector::Fixed(Arc::new(Mutex::new(Ok(Default::default()))));
        engine.with_network_pool(Default::default(), detector)
    }

    /// 启动假 Docker，返回它和连到它的 [`Engine`]。
    pub async fn start() -> (Self, Engine) {
        let fake = Self::default();
        let engine = fake.engine().await;
        (fake, engine)
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

    pub fn network_collisions(&self) -> u32 {
        self.lock().network_collisions
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
    /// 模拟主人手动删掉一个网络（或上一代核心清掉过）。
    pub fn remove_network(&self, name: &str) {
        let mut docker = self.lock();
        docker.endpoints.remove(name);
        docker.network_bodies.remove(name);
        docker.networks.remove(name);
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
        let networks = body["NetworkingConfig"]["EndpointsConfig"].as_object();
        for (network, endpoint) in networks.into_iter().flatten() {
            docker
                .endpoints
                .entry(network.clone())
                .or_default()
                .insert(name.to_owned(), endpoint.clone());
        }
        docker.created.insert(name.to_owned(), body);
    }

    /// 把容器接进网络，如核心升级前的容器。
    pub fn add_endpoint(&self, network: &str, container: &str, endpoint: Value) {
        self.lock()
            .endpoints
            .entry(network.to_owned())
            .or_default()
            .insert(container.to_owned(), endpoint);
    }

    /// 容器在网络里的端点。
    pub fn endpoint(&self, network: &str, container: &str) -> Option<Value> {
        self.lock().endpoints.get(network)?.get(container).cloned()
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

    /// 容器被创建过的次数（重建会计数）。
    pub fn creations(&self, container: &str) -> u32 {
        self.lock()
            .creation_count
            .get(container)
            .copied()
            .unwrap_or(0)
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

    pub fn set_start_state(&self, container: &str, state: Value) {
        self.lock().start_states.insert(container.to_owned(), state);
    }

    pub fn fail_starts(&self, message: Option<&str>) {
        self.lock().start_error = message.map(str::to_owned);
    }

    /// 容器里执行的命令由 `handler` 回应。
    pub fn on_exec(
        &self,
        handler: impl Fn(&str, &[String], &[u8]) -> (i64, String) + Send + Sync + 'static,
    ) {
        self.lock().exec_handler = Some(Arc::new(handler));
    }

    /// 接下来 `times` 次指定网段的建网络，Docker 回 `message`（403）。
    pub fn reject_pinned_networks(&self, times: u32, message: &str) {
        self.lock().pinned_network_rejection = Some((times, message.to_owned()));
    }

    /// 容器里执行的命令可分别回应标准输出和标准错误。
    pub fn on_exec_output(
        &self,
        handler: impl Fn(&str, &[String], &[u8]) -> (i64, String, String) + Send + Sync + 'static,
    ) {
        self.lock().exec_output_handler = Some(Arc::new(handler));
    }

    /// 之后镜像都不在本地、拉取时 Docker 回 `message`（500）；`None` 恢复成镜像都在本地。
    pub fn fail_pulls(&self, message: Option<&str>) {
        self.lock().pull_error = message.map(str::to_owned);
    }

    /// 同 [`Self::fail_pulls`]，但名字里有 `spared` 的镜像还在本地。
    pub fn fail_pulls_except(&self, spared: &str, message: &str) {
        let mut docker = self.lock();
        docker.pull_error = Some(message.to_owned());
        docker.pull_spared = Some(spared.to_owned());
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

    /// 放进一个核心容器：跑着的镜像 ID 和创建时的镜像引用（`Config.Image`）可以不一样，
    /// 模拟核心镜像的标签已被移走（#666）。
    pub fn add_core_container(&self, name: &str, image_id: &str, reference: &str) {
        let mut docker = self.lock();
        docker.containers.insert(name.to_owned(), Labels::new());
        docker.created.insert(
            name.to_owned(),
            json!({"Image": image_id, "Config": {"Image": reference}}),
        );
    }

    /// 忘掉一个镜像：查它和用它创建容器都回 404，像 containerd 存储下镜像记录被清掉后那样（#666）。
    pub fn forget_image(&self, image: &str) {
        self.lock().missing_images.insert(image.to_owned());
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

    /// 让 `GET /info` 报告这个名字（宿主主机名）。
    pub fn set_name(&self, name: &str) {
        self.lock().name = Some(name.to_owned());
    }

    /// 让 `GET /info` 回 `message`，直到传入 `None`。
    pub fn fail_info(&self, message: Option<&str>) {
        self.lock().info_error = message.map(str::to_owned);
    }

    pub fn set_https_proxy(&self, address: &str) {
        self.lock().https_proxy = Some(address.to_owned());
    }

    fn lock(&self) -> MutexGuard<'_, Resources> {
        self.resources.lock().expect("假 Docker 的锁中毒")
    }
    /// 发一条 Docker 容器 OOM 事件，事件里的 ID 使用容器名，和 Docker inspect 一样可查询。
    pub fn emit_oom(&self, container: &str) {
        let _ = self.events.send(json!({
            "Type": "container",
            "Action": "oom",
            "Actor": {"ID": container},
        }));
    }

    /// 下次容器更新时返回 Docker 错误；`None` 恢复正常。
    pub fn fail_updates(&self, message: Option<&str>) {
        self.lock().update_error = message.map(str::to_owned);
    }
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
    if *method == Method::GET && path == "/events" {
        return events_response(fake.events.subscribe());
    }
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
        ["volumes", rest @ ..] => volumes(&mut docker, method, rest, query, body),
        _ => others(&mut docker, method, &parts, query),
    };
    reply.unwrap_or_else(|| {
        json_reply(
            StatusCode::NOT_FOUND,
            json!({"message": format!("假 Docker 没有 {call}")}),
        )
    })
}

/// 版本、镜像、exec 查询。
fn others(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
) -> Option<Response> {
    let reply = match (method, parts) {
        (&Method::GET, ["version"]) => json_reply(
            StatusCode::OK,
            json!({"Arch": "x86_64", "Version": "29.0.0"}),
        ),
        (&Method::GET, ["images", image @ .., "json"])
            if docker.pull_error.is_some()
                && !docker
                    .pull_spared
                    .as_ref()
                    .is_some_and(|spared| image.join("/").contains(spared.as_str())) =>
        {
            json_reply(StatusCode::NOT_FOUND, json!({"message": "No such image"}))
        }
        (&Method::GET, ["info"]) if docker.info_error.is_some() => json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": docker.info_error}),
        ),
        (&Method::GET, ["info"]) => {
            let runtimes = docker
                .runtimes
                .clone()
                .unwrap_or_else(|| vec!["runc".to_owned()]);
            let map: serde_json::Map<String, Value> = runtimes
                .into_iter()
                .map(|name| (name.clone(), json!({"path": name})))
                .collect();
            json_reply(
                StatusCode::OK,
                json!({"Name": docker.name, "Runtimes": map, "Architecture": "x86_64", "OSType": "linux", "NCPU": 32, "HttpsProxy": docker.https_proxy}),
            )
        }
        (&Method::GET, ["images", image @ .., "json"])
            if docker.missing_images.contains(&image.join("/")) =>
        {
            json_reply(StatusCode::NOT_FOUND, json!({"message": "No such image"}))
        }
        (&Method::GET, ["images", image @ .., "json"]) => {
            let check = docker.image_healthchecks.get(&image.join("/"));
            json_reply(StatusCode::OK, json!({"Config": {"Healthcheck": check}}))
        }
        (&Method::POST, ["images", "create"]) if docker.pull_error.is_some() => json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": docker.pull_error}),
        ),
        // 拉成功：像 Docker 一样一行一个 JSON，带进度，最后一条表示下完。每行隔一会儿，测试才看得到中间状态。
        (&Method::POST, ["images", "create"]) => {
            // 拉完的这个镜像就在本地了：之后查它、用它建容器都正常；没拉的还在不在看它原来在不在。
            // 核心拉的引用补了 `:latest`，`forget_image` 记的可能是不带标签的名字。
            if let Some(image) = query_param(query, "fromImage") {
                docker
                    .missing_images
                    .retain(|missing| *missing != image && format!("{missing}:latest") != image);
            }
            let lines = futures_util::stream::iter([
                "{\"status\":\"Downloading\",\"progressDetail\":{\"current\":40,\"total\":100},\"id\":\"a\"}",
                "{\"status\":\"Downloading\",\"progressDetail\":{\"current\":40,\"total\":100},\"id\":\"b\"}",
                "{\"status\":\"Pull complete\",\"id\":\"a\"}",
                "{\"status\":\"Downloading\",\"progressDetail\":{\"current\":100,\"total\":100},\"id\":\"b\"}",
                "{\"status\":\"Already exists\",\"id\":\"c\"}",
                "{\"status\":\"Pull complete\",\"id\":\"b\"}",
            ])
            .then(|line| async move {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                Ok::<_, std::io::Error>(Bytes::from(format!("{line}\n")))
            });
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(axum::body::Body::from_stream(lines))
                .unwrap()
        }
        (&Method::GET, ["exec", id, "json"]) if docker.execs.contains_key(*id) => {
            let code = docker.execs[*id].exit_code;
            json_reply(
                StatusCode::OK,
                json!({"ExitCode": code, "Running": code.is_none()}),
            )
        }
        _ => return None,
    };
    Some(reply)
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
