//! 核心在 Tuwunel 上的管理账号（ADR-0009 的验证结果 M0-2）：用注册共享密钥注册一次
//! （Synapse 兼容的 `POST /_synapse/admin/v1/register`，`admin: true`），访问令牌存在核心数据目录（0600）。
//! 令牌丢了或失效时，用核心签的 JWT 登录同一个账号拿新令牌：账号注册时就是管理员。

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use aws_lc_rs::hmac;
use hyper::Method;
use serde_json::json;

use super::api::{Api, MatrixError};
use super::issuer::Issuer;
use crate::secret::random_token;

/// 管理账号的用户名。
pub const ADMIN_LOCALPART: &str = wonton_protocol::CORE_LOCALPART;

/// Tuwunel 自己的服务器用户（v1.9.4 配置项 `server_user_localpart` 的默认值，核心不改它）。JWT 登录能进这个账号，
/// 主人和 Bot 都不能叫这个名字。
pub const SERVER_USER_LOCALPART: &str = "conduit";

/// 读出能用的管理令牌，没有或失效时注册（首次）或重新登录，并保存。
pub async fn admin_token(
    api: &Api,
    issuer: &Issuer,
    shared_secret: &str,
    path: &Path,
) -> Result<String, MatrixError> {
    if let Some(token) = read_token(path)?
        && api.whoami(&token).await?.is_some()
    {
        return Ok(token);
    }
    let token = match register(api, shared_secret).await {
        Ok(token) => token,
        Err(e) if e.errcode() == "M_USER_IN_USE" => {
            let jwt = issuer.login_token(ADMIN_LOCALPART);
            api.login(&jwt, "Wonton 核心").await?.access_token
        }
        Err(e) => return Err(e),
    };
    save_token(path, &token)?;
    Ok(token)
}

/// 共享密钥注册：先取一次性的 nonce，再用 HMAC-SHA1 签 `nonce\0用户名\0密码\0admin`。密码随机生成、不保存：
/// Tuwunel 不接受密码登录，管理账号之后也用 JWT 登录。
async fn register(api: &Api, shared_secret: &str) -> Result<String, MatrixError> {
    let path = "/_synapse/admin/v1/register";
    let nonce = api
        .json(Method::GET, path, None, None, "取注册 nonce")
        .await?;
    let nonce = nonce["nonce"]
        .as_str()
        .ok_or_else(|| MatrixError::Invalid("注册接口没有返回 nonce".to_owned()))?;
    let password = random_token();
    let body = json!({
        "nonce": nonce,
        "username": ADMIN_LOCALPART,
        "password": password,
        "admin": true,
        "mac": registration_mac(shared_secret, nonce, ADMIN_LOCALPART, &password),
    });
    let value = api
        .json(Method::POST, path, None, Some(&body), "注册管理账号")
        .await?;
    value["access_token"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| MatrixError::Invalid("注册接口没有返回访问令牌".to_owned()))
}

/// Synapse 的共享密钥注册签名，十六进制小写。
fn registration_mac(shared_secret: &str, nonce: &str, user: &str, password: &str) -> String {
    let key = hmac::Key::new(
        hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        shared_secret.as_bytes(),
    );
    let message = [nonce, user, password, "admin"].join("\0");
    hmac::sign(&key, message.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read_token(path: &Path) -> Result<Option<String>, MatrixError> {
    match fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => Ok(Some(text.trim().to_owned())),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(storage(path, &e)),
    }
}

/// 先写临时文件再改名，权限 0600。
fn save_token(path: &Path, token: &str) -> Result<(), MatrixError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| storage(parent, &e))?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| storage(&tmp, &e))?;
    file.write_all(token.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| storage(&tmp, &e))?;
    fs::rename(&tmp, path).map_err(|e| storage(path, &e))
}

fn storage(path: &Path, error: &std::io::Error) -> MatrixError {
    MatrixError::Storage(format!(
        "读写 Matrix 管理令牌 {} 失败：{error}",
        path.display()
    ))
}
