//! 假 Docker 的 HTTP 路由与分发：把请求按资源和动词交给各模块。
//!
//! `handle` 是 axum 的落点，`serve` 按资源分发，`api_path` 去掉 bollard 的版本前缀。

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, Uri};
use axum::response::Response;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};

use super::containers::containers;
use super::events::events_response;
use super::exec::start_exec;
use super::images::distribution;
use super::networks::networks;
use super::volumes::volumes;
use super::{FakeDocker, Resources, json_reply, query_param};

pub(super) async fn handle(State(fake): State<FakeDocker>, request: Request) -> Response {
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
    let committed = {
        let mut docker = fake.lock();
        let matches = method == Method::POST
            && api_path(&uri) == "/containers/create"
            && docker.committed_create.as_ref().is_some_and(|(name, _)| {
                query_param(uri.query().unwrap_or_default(), "name").as_ref() == Some(name)
            });
        matches.then(|| docker.committed_create.take()).flatten()
    };
    let pull_delay = fake.lock().pull_delay;
    if method == Method::POST
        && api_path(&uri) == "/images/create"
        && let Some(delay) = pull_delay
    {
        tokio::time::sleep(delay).await;
    }
    let response = serve(&fake, &method, &uri, &body);
    if let Some((name, hold)) = committed
        && response.status() == StatusCode::CREATED
    {
        {
            let mut docker = fake.lock();
            docker
                .states
                .insert(name.clone(), json!({"Status": "created", "Running": false}));
            docker.restartable.insert(name, None);
        }
        hold.reached.notify_one();
        hold.released.acquire().await.ok();
    }
    response
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
    let arch = docker.arch.clone().unwrap_or_else(|| "x86_64".to_owned());
    let reply = match (method, parts) {
        (&Method::GET, ["version"]) => {
            json_reply(StatusCode::OK, json!({"Arch": arch, "Version": "29.0.0"}))
        }
        (&Method::GET, ["distribution", image @ .., "json"]) => {
            return distribution(docker, &image.join("/"));
        }
        // 拉不到镜像时本地也没有这张镜像：查镜像要报没有，安装才会走到拉镜像那步。
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
                json!({"Name": docker.name, "Runtimes": map, "Architecture": arch, "OSType": "linux", "NCPU": 32, "HttpsProxy": docker.https_proxy}),
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
