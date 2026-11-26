//! 测试用的假 Docker Engine API：只实现插件安装、核对、卸载和回滚用到的端点，记下现存的容器、网络和卷
//! （连同标签），供测试断言对 Docker 的副作用。容器一经创建默认报告为运行中且健康；显式 stop 和
//! committed-create 故障会改变状态，测试也可以按容器名指定退出、健康检查等状态。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, broadcast};

use self::routes::handle;
use super::docker::Engine;
use super::names::LABEL_PLUGIN;

mod containers;
mod events;
mod exec;
mod files;
mod images;
mod networks;
mod routes;
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
    /// 服务端真的把命令跑起来过；创建了没启动的是 `false`。
    started: bool,
    exit_code: Option<i64>,
    /// 创建顺序：exec id 是随机串，按它找“最后一条”会挑错。
    created: u64,
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
    /// 标了删除、删除还没做完的容器 → 再被 inspect 几次才连同端点一起没：接出网络被 Docker 拒绝，
    /// 等够了次数容器和端点一起消失，模拟 Docker 删容器的异步过程。
    removing: BTreeMap<String, u32>,
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
    /// 容器名 → 被 inspect 的次数。
    inspect_count: BTreeMap<String, u32>,
    /// 容器名 → 端点没掉那一刻 `writes` 的条数：断言删网络发生在端点消失之后用。
    removed_at: BTreeMap<String, usize>,
    /// 写进容器的归档：容器名 → 归档内容。
    archives: BTreeMap<String, Vec<u8>>,
    /// 容器里能经归档接口读到的路径：（容器名，路径）→ 内容。
    files: BTreeMap<(String, String), FakeFile>,
    /// 按容器名指定的状态（Docker inspect 里的 `State`），没指定的报告为运行中且健康。
    states: BTreeMap<String, Value>,
    start_states: BTreeMap<String, Value>,
    start_error: Option<String>,
    stop_error: Option<String>,
    create_error: Option<(String, String)>,
    kill_error: Option<(String, String)>,
    remove_error: Option<(String, String)>,
    /// 按容器名让 inspect（`GET /containers/{name}/json`）一直报错。
    inspect_error: Option<(String, String)>,
    signals: BTreeMap<String, u32>,
    /// 网络名 → 最近一次把容器接入它的请求（`EndpointConfig.Aliases` 等）。
    connections: BTreeMap<String, Value>,
    /// 每个网络最近一次主动接入（`connect`）的别名；创建时带的端点不算。
    aliases: BTreeMap<String, Vec<String>>,
    /// 网络名 → 容器名 → 它在网络里的端点（Docker 的 `EndpointSettings` 写法）。
    endpoints: BTreeMap<String, BTreeMap<String, Value>>,
    /// exec ID → 这次执行。按创建顺序编号在 `next_exec` 里单独记，不靠 id 排序。
    execs: BTreeMap<String, FakeExec>,
    /// 下一条 exec 的创建序号：`finish_last_exec` 按它挑“最后创建”，id 是随机串。
    next_exec: u64,
    /// 没设时命令都以 0 退出。
    exec_handler: Option<ExecHandler>,
    exec_output_handler: Option<ExecOutputHandler>,
    /// 设了时本地没有镜像，拉取时 Docker 回这个错误；没设时镜像都在本地。
    pull_error: Option<String>,
    /// 名字里有这段的镜像不受 `pull_error` 影响，还在本地。
    pull_spared: Option<String>,
    /// 拉镜像的请求拖这么久才回，模拟拉不完（客户端先超时）。
    pull_delay: Option<Duration>,
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
    /// 下一个同名容器先创建为 Created，再扣住 HTTP 回答。
    committed_create: Option<(String, Hold)>,
    /// 下一次 exec 启动回这个错误，以及服务端是否已经把命令跑起来了（结局未知，见 `fail_next_exec_start`）。
    exec_start_error: Option<(String, bool)>,
    /// 由 stop 或故障注入设置的状态，下次正常 start 可以恢复；set_state 不受影响。
    restartable: BTreeMap<String, Option<Value>>,
    /// `GET /version`、`/info` 报告的架构；没设时 `x86_64`。
    arch: Option<String>,
    /// 镜像 → 平台列表（`GET /distribution/{镜像}/json`）。没设的镜像报一个 linux/amd64 的单清单。
    platforms: BTreeMap<String, FakePlatforms>,
    /// 设了时 `/distribution/` 回这个错误（离线、仓库拒绝）。
    platform_error: Option<String>,
}

/// 假 Docker 对镜像平台列表的回答。
#[derive(Debug, Clone, Default)]
pub struct FakePlatforms {
    pub architectures: Vec<String>,
    /// 单个清单时的压缩后大小；多架构列表没有。
    pub size: Option<u64>,
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

    /// 下次创建这个容器时留下真实资源，但在放行前不回答客户端。
    pub fn hold_committed_create(&self, name: &str) -> Hold {
        let hold = Hold {
            call: "POST /containers/create".to_owned(),
            reached: Arc::new(Notify::new()),
            released: Arc::new(Semaphore::new(0)),
        };
        self.lock().committed_create = Some((name.to_owned(), hold.clone()));
        hold
    }

    pub fn writes(&self) -> Vec<String> {
        self.lock().writes.clone()
    }

    /// 让 `GET /version`、`/info` 报告这个宿主架构（`aarch64` 等）；没设时 `x86_64`。
    pub fn set_arch(&self, arch: &str) {
        self.lock().arch = Some(arch.to_owned());
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

    /// 暂停回应（请求照收），直到丢掉返回的守卫，比如让卸载停在半路。
    pub async fn pause(&self) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.gate.clone().write_owned().await
    }

    fn lock(&self) -> MutexGuard<'_, Resources> {
        self.resources.lock().expect("假 Docker 的锁中毒")
    }
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
