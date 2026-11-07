//! 假 Docker 的卷：建、查、删和按标签列表。

use std::collections::BTreeMap;

use axum::body::Bytes;
use axum::http::{Method, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};

use super::{Resources, gone, json_reply, labeled, labels_of};

type Labels = BTreeMap<String, String>;

pub(super) fn volumes(
    docker: &mut Resources,
    method: &Method,
    parts: &[&str],
    query: &str,
    raw: &Bytes,
) -> Option<Response> {
    let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let reply = match (method, parts) {
        (&Method::POST, ["create"]) => {
            let name = body["Name"].as_str().unwrap_or_default().to_owned();
            let labels = labels_of(&body);
            let reply = volume(&name, &labels);
            docker.volumes.insert(name, labels);
            json_reply(StatusCode::CREATED, reply)
        }
        (&Method::GET, [name]) if docker.volumes.contains_key(*name) => {
            json_reply(StatusCode::OK, volume(name, &docker.volumes[*name]))
        }
        (&Method::DELETE, [name]) => gone(docker.volumes.remove(*name).is_some()),
        (&Method::GET, []) => {
            let list: Vec<Value> = labeled(&docker.volumes, query)
                .map(|(n, labels)| volume(n, labels))
                .collect();
            json_reply(StatusCode::OK, json!({"Volumes": list, "Warnings": []}))
        }
        _ => return None,
    };
    Some(reply)
}

fn volume(name: &str, labels: &Labels) -> Value {
    json!({
        "Name": name, "Driver": "local", "Mountpoint": format!("/var/lib/docker/volumes/{name}"),
        "Labels": labels, "Options": {}, "Scope": "local"
    })
}
