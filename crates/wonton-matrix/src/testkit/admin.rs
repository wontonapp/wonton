//! 模拟核心的管理操作：经 Tuwunel 实现的 Synapse 兼容管理接口注册管理账号、吊销设备、停用账号。

use crate::{DeviceId, UserId};
use aws_lc_rs::hmac;
use matrix_sdk::reqwest::{Client, Method};
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
        let path = format!("/_synapse/admin/v2/users/{user_id}/devices");
        let response = self.call(Method::GET, &path, None).await;
        let devices = response["devices"].as_array().expect("应返回设备列表");
        devices
            .iter()
            .map(|device| device["device_id"].as_str().unwrap().to_owned())
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
