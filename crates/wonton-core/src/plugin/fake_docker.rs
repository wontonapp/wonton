//! 测试用的假 Docker Engine API：只实现插件安装、核对、卸载和回滚用到的端点，记下现存的容器和网络
//! （连同 `wonton.plugin` 标签），供测试断言对 Docker 的副作用。容器一经创建就报告为运行中且健康。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::docker::Engine;
use super::render::LABEL_PLUGIN;

#[derive(Clone, Default)]
pub struct FakeDocker {
    resources: Arc<Mutex<Resources>>,
}

#[derive(Default)]
struct Resources {
    /// 容器名 → 所属插件。
    containers: BTreeMap<String, String>,
    /// 网络名 → 所属插件。
    networks: BTreeMap<String, String>,
    /// 收到的写操作，按顺序，如 `POST /containers/x/start`。
    writes: Vec<String>,
    /// 写进容器的归档：容器名 → 归档内容。
    archives: BTreeMap<String, Vec<u8>>,
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
            .insert(name.to_owned(), plugin.to_owned());
    }

    pub fn containers(&self) -> Vec<String> {
        self.lock().containers.keys().cloned().collect()
    }

    pub fn networks(&self) -> Vec<String> {
        self.lock().networks.keys().cloned().collect()
    }

    pub fn writes(&self) -> Vec<String> {
        self.lock().writes.clone()
    }

    /// 最近一次写进这个容器的 tar 归档。
    pub fn archive(&self, container: &str) -> Option<Vec<u8>> {
        self.lock().archives.get(container).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, Resources> {
        self.resources.lock().expect("假 Docker 的锁中毒")
    }
}

async fn handle(State(fake): State<FakeDocker>, method: Method, uri: Uri, body: Bytes) -> Response {
    // bollard 的请求路径可能带 API 版本前缀（/v1.51/...）。
    let path = uri.path();
    let path = match path.strip_prefix("/v") {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => {
            rest.find('/').map_or("/", |i| &rest[i..])
        }
        _ => path,
    };
    let call = format!("{method} {path}");
    let query = uri.query().unwrap_or_default();
    let raw = body;
    let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let mut docker = fake.lock();
    if method != Method::GET {
        docker.writes.push(call.clone());
    }
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method, parts.as_slice()) {
        (Method::PUT, ["containers", name, "archive"]) if docker.containers.contains_key(*name) => {
            docker.archives.insert((*name).to_owned(), raw.to_vec());
            StatusCode::OK.into_response()
        }
        (Method::GET, ["version"]) => json_reply(StatusCode::OK, json!({"Arch": "x86_64"})),
        (Method::GET, ["images", .., "json"]) => json_reply(StatusCode::OK, json!({})),
        (Method::POST, ["networks", "create"]) => {
            let name = body["Name"].as_str().unwrap_or_default().to_owned();
            docker.networks.insert(name.clone(), plugin_label(&body));
            json_reply(StatusCode::CREATED, json!({"Id": name, "Warning": ""}))
        }
        (Method::DELETE, ["networks", name]) => gone(docker.networks.remove(*name).is_some()),
        (Method::GET, ["networks"]) => {
            let names = labeled(&docker.networks, query);
            let list: Vec<Value> = names.iter().map(|n| json!({"Name": n})).collect();
            json_reply(StatusCode::OK, json!(list))
        }
        (Method::POST, ["containers", "create"]) => {
            let name = url::form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == "name")
                .map(|(_, name)| name.into_owned())
                .unwrap_or_default();
            docker.containers.insert(name.clone(), plugin_label(&body));
            json_reply(StatusCode::CREATED, json!({"Id": name, "Warnings": []}))
        }
        (Method::POST, ["containers", name, "start" | "stop"]) => {
            gone(docker.containers.contains_key(*name))
        }
        (Method::DELETE, ["containers", name]) => gone(docker.containers.remove(*name).is_some()),
        (Method::GET, ["containers", name, "json"]) if docker.containers.contains_key(*name) => {
            json_reply(
                StatusCode::OK,
                json!({"Name": format!("/{name}"), "State": {
                    "Status": "running", "Running": true, "Health": {"Status": "healthy"}
                }}),
            )
        }
        (Method::GET, ["containers", "json"]) => {
            let names = labeled(&docker.containers, query);
            let list: Vec<Value> = names
                .iter()
                .map(|n| json!({"Names": [format!("/{n}")]}))
                .collect();
            json_reply(StatusCode::OK, json!(list))
        }
        (Method::GET, ["volumes"]) => {
            json_reply(StatusCode::OK, json!({"Volumes": [], "Warnings": []}))
        }
        _ => json_reply(
            StatusCode::NOT_FOUND,
            json!({"message": format!("假 Docker 没有 {call}")}),
        ),
    }
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

fn plugin_label(body: &Value) -> String {
    body["Labels"][LABEL_PLUGIN]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// 按查询里的 `filters={"label":["wonton.plugin=<id>"]}` 筛出资源名。
fn labeled(resources: &BTreeMap<String, String>, query: &str) -> Vec<String> {
    let wanted = url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == "filters")
        .and_then(|(_, filters)| serde_json::from_str::<Value>(&filters).ok())
        .and_then(|filters| filters["label"][0].as_str().map(str::to_owned))
        .and_then(|label| {
            label
                .strip_prefix(&format!("{LABEL_PLUGIN}="))
                .map(str::to_owned)
        });
    resources
        .iter()
        .filter(|(_, owner)| Some(owner.as_str()) == wanted.as_deref())
        .map(|(name, _)| name.clone())
        .collect()
}
