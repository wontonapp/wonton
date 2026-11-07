//! 模拟核心的管理操作：经 Tuwunel 实现的 Synapse 兼容管理接口注册管理账号、吊销设备、停用账号。

use crate::{DeviceId, UserId};
use aws_lc_rs::hmac;
use matrix_sdk::reqwest::{Client, Method, RequestBuilder};
use serde_json::{Value, json};

use super::random_hex;

pub struct Admin {
    http: Client,
    base_url: String,
    access_token: String,
}

impl Admin {
    /// 核心用共享密钥注册自己的管理账号（`POST /_synapse/admin/v1/register`），不需要开放注册。
    pub async fn register(base_url: &str, shared_secret: &str) -> Self {
        let http = Client::new();
        let path = "/_synapse/admin/v1/register";
        let nonce = request(&http, base_url, None, Method::GET, path, None).await["nonce"]
            .as_str()
            .expect("注册接口应返回 nonce")
            .to_owned();
        let (username, password) = ("wonton-core", random_hex(16));
        let message = [nonce.as_str(), username, &password, "admin"].join("\0");
        let key = hmac::Key::new(
            hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            shared_secret.as_bytes(),
        );
        let mac = hex(hmac::sign(&key, message.as_bytes()).as_ref());
        let body = json!({
            "nonce": nonce,
            "username": username,
            "password": password,
            "admin": true,
            "mac": mac,
        });
        let response = request(&http, base_url, None, Method::POST, path, Some(body)).await;
        let access_token = response["access_token"]
            .as_str()
            .expect("应返回访问令牌")
            .to_owned();
        Self {
            http,
            base_url: base_url.to_owned(),
            access_token,
        }
    }

    /// 吊销一台设备：删除设备及其访问令牌和刷新令牌。
    pub async fn delete_device(&self, user_id: &UserId, device_id: &DeviceId) {
        let path = format!("/_synapse/admin/v2/users/{user_id}/devices/{device_id}");
        self.call(Method::DELETE, &path, None).await;
    }

    /// 用户当前的设备 ID。
    pub async fn device_ids(&self, user_id: &UserId) -> Vec<String> {
        self.devices(user_id)
            .await
            .into_iter()
            .map(|(device_id, _)| device_id)
            .collect()
    }

    /// 用户的设备连同各自的显示名。
    pub async fn devices(&self, user_id: &UserId) -> Vec<(String, Option<String>)> {
        let path = format!("/_synapse/admin/v2/users/{user_id}/devices");
        let response = self.call(Method::GET, &path, None).await;
        let devices = response["devices"].as_array().expect("应返回设备列表");
        devices
            .iter()
            .map(|device| {
                (
                    device["device_id"].as_str().unwrap().to_owned(),
                    device["display_name"].as_str().map(str::to_owned),
                )
            })
            .collect()
    }

    /// 停用账号（例如卸载一个 Bot）：所有设备下线，之后也不能再登录。
    pub async fn deactivate(&self, user_id: &UserId) {
        let path = format!("/_synapse/admin/v1/deactivate/{user_id}");
        self.call(Method::POST, &path, Some(json!({ "erase": false })))
            .await;
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Value {
        request(
            &self.http,
            &self.base_url,
            Some(&self.access_token),
            method,
            path,
            body,
        )
        .await
    }
}

async fn request(
    http: &Client,
    base_url: &str,
    access_token: Option<&str>,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Value {
    let mut request = http.request(method.clone(), format!("{base_url}{path}"));
    if let Some(token) = access_token {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let response = request
        .send()
        .await
        .unwrap_or_else(|error| panic!("{method} {path}：{error}"));
    let status = response.status();
    let bytes = response.bytes().await.expect("读取响应");
    assert!(
        status.is_success(),
        "{method} {path} 返回 {status}：{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 停用后的旧会话居然还建出了房间：用旧令牌裸打一次 whoami、一次 refresh，把服务器的
/// 状态码和返回体连同相对 `since`（停用接口返回时刻）的毫秒数写进 panic 信息。
///
/// 只在断言失败的分支调用，正常路径零开销。诊断要不要等、要不要重试都不在这里决定，
/// 服务器对旧令牌的回答原样进 panic，交给看日志的人判断。
pub async fn diagnose_live_session(
    base_url: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    since: std::time::Instant,
) -> ! {
    let whoami = raw_exchange(
        Client::new()
            .get(format!("{base_url}/_matrix/client/v3/account/whoami"))
            .bearer_auth(access_token),
        since,
        "whoami",
    )
    .await;
    let refresh = match refresh_token {
        Some(refresh_token) => {
            raw_exchange(
                Client::new()
                    .post(format!("{base_url}/_matrix/client/v3/refresh"))
                    .header("content-type", "application/json")
                    .body(serde_json::json!({ "refresh_token": refresh_token }).to_string()),
                since,
                "refresh",
            )
            .await
        }
        None => "refresh: session had no refresh token".to_owned(),
    };
    panic!(
        "the deactivated session still created a room; old tokens as the server sees them: {whoami}; {refresh}"
    );
}

/// 发一个裸请求，返回「相对 `since` 的毫秒数 + 状态码 + 返回体」的一行描述。
async fn raw_exchange(request: RequestBuilder, since: std::time::Instant, what: &str) -> String {
    let at_ms = since.elapsed().as_millis();
    match request.send().await {
        Ok(response) => {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body unreadable: {error}>"));
            format!("{what} at +{at_ms}ms: {status} {body}")
        }
        Err(error) => format!("{what} at +{at_ms}ms: request failed: {error}"),
    }
}
