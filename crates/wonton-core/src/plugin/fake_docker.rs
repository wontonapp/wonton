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

/// 容器里的一个路径。
#[derive(Clone)]
pub enum FakeFile {
    Regular(Vec<u8>),
    Directory,
}

#[derive(Default)]
struct Resources {
    /// 容器名 → 所属插件。
    containers: BTreeMap<String, String>,
    /// 容器名 → 创建时的环境变量。
    envs: BTreeMap<String, Vec<String>>,
    /// 网络名 → 所属插件。
    networks: BTreeMap<String, String>,
    /// 收到的写操作，按顺序，如 `POST /containers/x/start`。
    writes: Vec<String>,
    /// 写进容器的归档：容器名 → 归档内容。
    archives: BTreeMap<String, Vec<u8>>,
    /// 容器里能经归档接口读到的路径：（容器名，路径）→ 内容。
    files: BTreeMap<(String, String), FakeFile>,
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

    /// 容器最近一次创建时的环境变量。
    pub fn env(&self, container: &str) -> Vec<String> {
        self.lock().envs.get(container).cloned().unwrap_or_default()
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
    let mut docker = fake.lock();
    if method != Method::GET {
        docker.writes.push(call.clone());
    }
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let reply = match parts.as_slice() {
        ["containers", rest @ ..] => containers(&mut docker, &method, rest, query, &body),
        _ => others(&mut docker, &method, &parts, query, &body),
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
        (&Method::POST, ["create"]) => {
            let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
            let name = query_param(query, "name").unwrap_or_default();
            let env = body["Env"].as_array().into_iter().flatten();
            let env = env.filter_map(|e| e.as_str().map(str::to_owned)).collect();
            docker.envs.insert(name.clone(), env);
            docker.containers.insert(name.clone(), plugin_label(&body));
            json_reply(StatusCode::CREATED, json!({"Id": name, "Warnings": []}))
        }
        (&Method::POST, [name, "start" | "stop"]) => gone(exists(name)),
        (&Method::DELETE, [name]) => gone(docker.containers.remove(*name).is_some()),
        (&Method::GET, [name, "json"]) if exists(name) => json_reply(
            StatusCode::OK,
            json!({"Name": format!("/{name}"), "State": {
                "Status": "running", "Running": true, "Health": {"Status": "healthy"}
            }}),
        ),
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
            let names = labeled(&docker.containers, query);
            let list: Vec<Value> = names
                .iter()
                .map(|n| json!({"Names": [format!("/{n}")]}))
                .collect();
            json_reply(StatusCode::OK, json!(list))
        }
        _ => return None,
    };
    Some(reply)
}

/// 版本、镜像、网络、卷。
fn others(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let reply = match (method, parts) {
        (&Method::GET, ["version"]) => json_reply(StatusCode::OK, json!({"Arch": "x86_64"})),
        (&Method::GET, ["images", .., "json"]) => json_reply(StatusCode::OK, json!({})),
        (&Method::POST, ["networks", "create"]) => {
            let name = body["Name"].as_str().unwrap_or_default().to_owned();
            docker.networks.insert(name.clone(), plugin_label(&body));
            json_reply(StatusCode::CREATED, json!({"Id": name, "Warning": ""}))
        }
        (&Method::DELETE, ["networks", name]) => gone(docker.networks.remove(*name).is_some()),
        (&Method::GET, ["networks"]) => {
            let names = labeled(&docker.networks, query);
            let list: Vec<Value> = names.iter().map(|n| json!({"Name": n})).collect();
            json_reply(StatusCode::OK, json!(list))
        }
        (&Method::GET, ["volumes"]) => {
            json_reply(StatusCode::OK, json!({"Volumes": [], "Warnings": []}))
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

fn plugin_label(body: &Value) -> String {
    body["Labels"][LABEL_PLUGIN]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// 按查询里的 `filters={"label":["wonton.plugin=<id>"]}` 筛出资源名。
fn labeled(resources: &BTreeMap<String, String>, query: &str) -> Vec<String> {
    let wanted = query_param(query, "filters")
        .and_then(|filters| serde_json::from_str::<Value>(&filters).ok())
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
