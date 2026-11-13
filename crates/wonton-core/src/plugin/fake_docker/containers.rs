//! `/containers...` 的端点：创建、更新、启停、删除、查询、等待、日志、exec、归档。

use axum::body::Bytes;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::networks::{endpoint_settings, published, running, state_name};
use super::update::update_container;
use super::{FakeExec, Resources, archive_of, gone, json_reply, labeled, labels_of, query_param};

pub(super) fn containers(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    if let Some(reply) = container_details(docker, method, parts, query, raw) {
        return Some(reply);
    }
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
            create(docker, &name, body)
        }
        (&Method::POST, [name, "update"]) if exists(name) => update_container(docker, name, raw),
        (&Method::POST, [name, "start"]) if exists(name) => start_container(docker, name),
        (&Method::POST, [name, "stop"]) if exists(name) => {
            if let Some(message) = &docker.stop_error {
                return Some(json_reply(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"message": message}),
                ));
            }
            // 和 Docker 一样：停掉的容器交回网络端点（固定地址让出来），inspect 显示 exited。
            let previous = docker.states.insert(
                name.to_string(),
                json!({"Status": "exited", "Running": false, "ExitCode": 0}),
            );
            docker
                .restartable
                .entry(name.to_string())
                .or_insert(previous);
            StatusCode::OK.into_response()
        }
        (&Method::POST, [name, "kill"]) if exists(name) => gone(true),
        (&Method::POST, [name, "start" | "stop"]) => gone(exists(name)),
        (&Method::DELETE, [name])
            if docker
                .remove_error
                .as_ref()
                .is_some_and(|(target, _)| target == name) =>
        {
            let (_, message) = docker.remove_error.as_ref().unwrap();
            json_reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"message": message}),
            )
        }
        (&Method::DELETE, [name]) => {
            if docker.restartable.remove(*name).is_some() {
                docker.states.remove(*name);
            }
            for endpoints in docker.endpoints.values_mut() {
                endpoints.remove(*name);
            }
            // 容器没了，它的运行状态清掉：同名的下一个容器不该继承上一个的退出态。start_states 的
            // 覆盖保留——它是测试按名字预置的"这个容器起成什么样"，重建后照样适用。
            docker.states.remove(*name);
            gone(docker.containers.remove(*name).is_some())
        }
        (&Method::GET, [name, "json"]) if exists(name) => inspect_container(docker, name),
        (&Method::POST, [name, "wait"]) if exists(name) => {
            let code = docker.outputs.get(*name).map_or(0, |o| o.0);
            json_reply(StatusCode::OK, json!({"StatusCode": code}))
        }
        (&Method::GET, ["json"]) => list_containers(docker, query),
        _ => return None,
    };
    Some(reply)
}

fn container_details(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    let exists = |name: &str| docker.containers.contains_key(name);
    let reply = match (method, parts) {
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
        _ => return None,
    };
    Some(reply)
}

/// Docker 在 start 时才分配地址：同网络里别的运行中容器已经占着同一个固定 IP，start 报这句
/// （真 Docker 29.7.2 原话，复审 451 实测）。返回撞上的容器名。
fn address_conflict(docker: &Resources, name: &str) -> Option<String> {
    for (network, endpoints) in &docker.endpoints {
        let Some(mine) = endpoints.get(name) else {
            continue;
        };
        let Some(wanted) = mine["IPAMConfig"]["IPv4Address"].as_str() else {
            continue;
        };
        for (other, endpoint) in endpoints {
            if other == name || endpoint["IPAMConfig"]["IPv4Address"].as_str() != Some(wanted) {
                continue;
            }
            if running(&docker.states, other) {
                return Some(other.clone());
            }
        }
        let _ = network;
    }
    None
}

/// 创建容器。镜像记录被清掉时 Docker 对创建回 404（#666），容器不会留下来。
fn create(docker: &mut Resources, name: &str, body: Value) -> Response {
    if docker
        .create_error
        .as_ref()
        .is_some_and(|(target, _)| target == name)
        && let Some((_, message)) = docker.create_error.take()
    {
        return json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": message}),
        );
    }
    if let Some(image) = body["Image"].as_str()
        && docker.missing_images.contains(image)
    {
        return json_reply(
            StatusCode::NOT_FOUND,
            json!({"message": format!("No such image: {image}")}),
        );
    }
    docker.archives.remove(name);
    docker.containers.insert(name.to_owned(), labels_of(&body));
    let networks = body["NetworkingConfig"]["EndpointsConfig"].as_object();
    for (network, endpoint) in networks.into_iter().flatten() {
        docker
            .endpoints
            .entry(network.clone())
            .or_default()
            .insert(name.to_owned(), endpoint.clone());
        // 创建时带的端点也是这个网络最近一次的接入（GatewayPriority 之类的断言看它）；和已有的合并。
        super::networks::record_endpoint(docker, network, endpoint);
    }
    // 和 Docker 一样：`NetworkMode` 写网络名时，容器接在这个网络上。
    if let Some(mode) = body["HostConfig"]["NetworkMode"].as_str()
        && docker.networks.contains_key(mode)
        && !docker
            .endpoints
            .get(mode)
            .is_some_and(|e| e.contains_key(name))
    {
        docker
            .endpoints
            .entry(mode.to_owned())
            .or_default()
            .insert(name.to_owned(), json!({}));
    }
    docker.created.insert(name.to_owned(), body);
    *docker.creation_count.entry(name.to_owned()).or_default() += 1;
    json_reply(StatusCode::CREATED, json!({"Id": name, "Warnings": []}))
}

fn start_container(docker: &mut Resources, name: &str) -> Response {
    if let Some(message) = &docker.start_error {
        return json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": message}),
        );
    }
    if let Some(other) = address_conflict(docker, name) {
        // start 失败的容器停在 created 态：不运行、不占地址。
        docker.states.insert(
            name.to_string(),
            json!({"Status": "created", "Running": false}),
        );
        return json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": format!("failed to set up container networking: Address already in use: {other} holds the fixed address")}),
        );
    }
    if let Some(state) = docker.start_states.get(name).cloned() {
        docker.states.insert(name.to_owned(), state);
    } else if let Some(previous) = docker.restartable.remove(name) {
        if let Some(state) = previous {
            docker.states.insert(name.to_owned(), state);
        } else {
            docker.states.remove(name);
        }
    } else if docker
        .states
        .get(name)
        .is_some_and(|s| s["Status"] == "created")
    {
        // 只有"建了没起起来"的标记要清掉；测试预置的状态（退出、不健康）照旧保留——真 Docker 里
        // start 后立刻退出或起不健康是常有的事，安装的健康门禁靠它模拟。
        docker.states.remove(name);
    }
    gone(true)
}

fn inspect_container(docker: &Resources, name: &str) -> Response {
    let healthy = json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}});
    let mut state = docker.states.get(name).cloned().unwrap_or(healthy);
    state
        .as_object_mut()
        .unwrap()
        .entry("StartedAt")
        .or_insert_with(|| json!("2026-01-01T00:00:00Z"));
    let created = docker.created.get(name).cloned().unwrap_or_default();
    let image = created["Image"].clone();
    // 创建请求里单独写了 `Config.Image` 时（`add_core_container`），inspect 的两处镜像就不一样。
    let config_image = created["Config"]["Image"]
        .as_str()
        .map(|_| created["Config"]["Image"].clone())
        .unwrap_or(image.clone());
    let networks: serde_json::Map<String, Value> = docker
        .endpoints
        .iter()
        .filter_map(|(network, endpoints)| {
            let mut settings = endpoint_settings(endpoints.get(name)?);
            if !running(&docker.states, name) {
                settings["IPAddress"] = json!("");
            }
            Some((network.clone(), settings))
        })
        .collect();
    json_reply(
        StatusCode::OK,
        json!({"Id": name, "Name": format!("/{name}"), "State": state, "Image": image, "RestartCount": 0,
               "Config": {"Image": config_image, "Labels": created["Labels"], "Env": created["Env"],
                          "User": created["User"], "Healthcheck": created["Healthcheck"]},
               "HostConfig": created["HostConfig"],
               "NetworkSettings": {"Networks": networks}}),
    )
}

fn list_containers(docker: &Resources, query: &str) -> Response {
    let network = query_param(query, "filters")
        .and_then(|filters| serde_json::from_str::<Value>(&filters).ok())
        .and_then(|filters| filters["network"][0].as_str().map(str::to_owned));
    let on_network = |container: &str| match &network {
        Some(network) => docker
            .endpoints
            .get(network)
            .is_some_and(|endpoints| endpoints.contains_key(container)),
        None => true,
    };
    let list: Vec<Value> = labeled(&docker.containers, query)
        .filter(|(n, _)| on_network(n))
        .map(|(n, labels)| {
            json!({
                "Names": [format!("/{n}")],
                "Labels": labels,
                "State": state_name(&docker.states, n),
                "Ports": published(&docker.created, &docker.states, n),
            })
        })
        .collect();
    json_reply(StatusCode::OK, json!(list))
}
