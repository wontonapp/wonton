//! 假 Docker 的网络：建网络、接入、查询和端点。

use std::collections::BTreeMap;

use axum::body::Bytes;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::{Resources, gone, json_reply, labeled, labels_of};

/// 还有要拒绝的次数时，拒绝建网络：指定网段的和让 Docker 分配网段的分开计数。
fn reject_create(docker: &mut Resources, body: &Value) -> Option<Response> {
    let slot = if body["IPAM"]["Config"][0]["Subnet"].is_string() {
        &mut docker.pinned_network_rejection
    } else {
        &mut docker.auto_network_rejection
    };
    let (times, message) = slot.as_mut()?;
    if *times == 0 {
        return None;
    }
    *times -= 1;
    Some(json_reply(
        StatusCode::FORBIDDEN,
        json!({"message": message.clone()}),
    ))
}

/// `/networks...` 的端点。
pub(super) fn networks(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    if let (&Method::POST, ["create"]) = (method, parts)
        && let Some(reply) = reject_create(docker, &body)
    {
        return Some(reply);
    }
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
            let container = body["Container"].as_str().unwrap_or_default().to_owned();
            let endpoints = docker.endpoints.entry((*name).to_owned()).or_default();
            if found && *action == "connect" {
                endpoints.insert(container, body["EndpointConfig"].clone());
                docker.connections.insert((*name).to_owned(), body);
            } else if found {
                endpoints.remove(&container);
            }
            if found {
                StatusCode::OK.into_response()
            } else {
                gone(false)
            }
        }
        (&Method::GET, [name]) if docker.networks.contains_key(*name) => {
            let created = &docker.network_bodies[*name];
            let containers: serde_json::Map<String, Value> = docker
                .endpoints
                .get(*name)
                .into_iter()
                .flatten()
                .filter(|(container, _)| running(&docker.states, container))
                .map(|(container, endpoint)| {
                    let address = endpoint_settings(endpoint)["IPAddress"].clone();
                    let address = format!("{}/24", address.as_str().unwrap_or_default());
                    let resource = json!({"Name": container, "IPv4Address": address});
                    (format!("id-{container}"), resource)
                })
                .collect();
            json_reply(
                StatusCode::OK,
                json!({
                    "Name": name, "Driver": "bridge", "Internal": created["Internal"],
                    "Options": created["Options"], "Labels": docker.networks[*name],
                    "IPAM": {"Config": [ipam_config(created)]}, "Containers": containers,
                }),
            )
        }
        (&Method::DELETE, [name])
            if docker
                .endpoints
                .get(*name)
                .is_some_and(|e| e.keys().any(|c| running(&docker.states, c))) =>
        {
            json_reply(
                StatusCode::FORBIDDEN,
                json!({"message": format!("error while removing network: network {name} has active endpoints")}),
            )
        }
        (&Method::DELETE, [name]) => {
            // 停着的容器挡不住删网络，之后再也接不回这个网络。
            docker.endpoints.remove(*name);
            docker.network_bodies.remove(*name);
            gone(docker.networks.remove(*name).is_some())
        }
        (&Method::GET, []) => {
            let list: Vec<Value> = labeled(&docker.networks, query)
                .map(|(n, labels)| {
                    let created = &docker.network_bodies[n];
                    json!({"Name": n, "Id": n, "Driver": "bridge", "Labels": labels,
                           "Options": created["Options"],
                           "IPAM": {"Config": [ipam_config(created)]}})
                })
                .collect();
            json_reply(StatusCode::OK, json!(list))
        }
        _ => return None,
    };
    Some(reply)
}

/// 网络的 IPAM：建的时候指定了就用它，没指定时是假 Docker 分配的网段。
fn ipam_config(created: &Value) -> Value {
    let default = json!({"Subnet": "172.30.0.0/24", "Gateway": "172.30.0.1"});
    let config = created["IPAM"]["Config"].get(0).cloned().unwrap_or(default);
    json!({"Subnet": config["Subnet"], "Gateway": config["Gateway"], "IPRange": config["IPRange"]})
}

/// 和 Docker 一样：网络的查询接口不列停着的容器，它们的端点也没有地址。
pub(super) fn running(states: &BTreeMap<String, Value>, container: &str) -> bool {
    states.get(container).is_none_or(|s| s["Running"] != false)
}

/// 容器 inspect 里的端点：没指定地址的报成 127.0.0.1，测试在本机起假的插件网页。
pub(super) fn endpoint_settings(endpoint: &Value) -> Value {
    let address = endpoint["IPAMConfig"]["IPv4Address"]
        .as_str()
        .unwrap_or("127.0.0.1");
    json!({"IPAddress": address, "Aliases": endpoint["Aliases"], "GwPriority": endpoint["GwPriority"]})
}
