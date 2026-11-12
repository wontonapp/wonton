//! 调 Tuwunel：客户端接口和 Synapse 兼容的管理接口，都是 JSON。错误按原因区分：Tuwunel 不可达，或者它拒绝了
//! 请求（带上 Matrix 的 `errcode`）。

use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode, header};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use crate::http::{HttpClient, RequestBody};

const TIMEOUT: Duration = Duration::from_secs(30);

/// 登录令牌类型：Tuwunel 的 JWT 登录。
pub const JWT_LOGIN_TYPE: &str = "org.matrix.login.jwt";

#[derive(Debug, thiserror::Error)]
pub enum MatrixError {
    #[error("Tuwunel 不可达（{context}）：{message}")]
    Unavailable { context: String, message: String },
    #[error("Tuwunel 拒绝了请求（{context}）：HTTP {status} {errcode}：{message}")]
    Rejected {
        context: String,
        status: u16,
        errcode: String,
        message: String,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Storage(String),
}

impl MatrixError {
    /// Matrix 的错误码，Tuwunel 不可达时为空。
    pub fn errcode(&self) -> &str {
        match self {
            Self::Rejected { errcode, .. } => errcode,
            _ => "",
        }
    }
}

/// 一次 JWT 登录的结果：新设备和它的访问令牌。`Debug` 不打印令牌。
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Login {
    pub user_id: String,
    pub device_id: String,
    pub access_token: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}

pub struct Api {
    http: HttpClient,
    /// `http://主机:端口`。
    base: String,
}

impl Api {
    pub fn new(http: HttpClient, base: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// 调一个返回 JSON 的接口，请求体是 JSON 或没有；`context` 写进错误里，说明核心当时在做什么。
    pub async fn json(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
        context: &str,
    ) -> Result<Value, MatrixError> {
        let body = body.map_or(RequestBody::Empty, RequestBody::Json);
        self.call(method, path, token, body, context).await
    }

    /// 上传文件到媒体库，返回 `mxc://` 地址。
    pub async fn upload(
        &self,
        token: &str,
        content_type: &str,
        filename: &str,
        data: Vec<u8>,
    ) -> Result<String, MatrixError> {
        let path = format!("/_matrix/media/v3/upload?filename={}", encode(filename));
        let body = RequestBody::Bytes { content_type, data };
        let value = self
            .call(Method::POST, &path, Some(token), body, "上传文件")
            .await?;
        value["content_uri"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("上传接口没有返回 content_uri".to_owned()))
    }

    /// 成功响应只保留上限加一字节，超限仍交给媒体检查归类为 TooLarge。
    pub async fn download(
        &self,
        token: &str,
        server: &str,
        id: &str,
    ) -> Result<Vec<u8>, MatrixError> {
        let path = format!(
            "/_matrix/client/v1/media/download/{}/{}",
            encode(server),
            encode(id)
        );
        let request = Request::get(format!("{}{path}", self.base))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .map_err(|error| MatrixError::Invalid(error.to_string()))?;
        let exchange = async {
            let response = self
                .http
                .send(request)
                .await
                .map_err(|error| error.to_string())?;
            let status = response.status();
            let limit = if status.is_success() {
                wonton_protocol::MAX_AVATAR_BYTES as usize + 1
            } else {
                1 << 20
            };
            let mut body = response.into_body();
            let mut bytes = Vec::new();
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|error| error.to_string())?;
                if let Some(data) = frame.data_ref() {
                    let remaining = limit - bytes.len();
                    bytes.extend_from_slice(&data[..data.len().min(remaining)]);
                    if bytes.len() == limit {
                        break;
                    }
                }
            }
            Ok::<_, String>((status, bytes))
        };
        let (status, bytes) = tokio::time::timeout(TIMEOUT, exchange)
            .await
            .map_err(|_| MatrixError::Unavailable {
                context: "下载头像".into(),
                message: "请求超时".into(),
            })?
            .map_err(|message| MatrixError::Unavailable {
                context: "下载头像".into(),
                message,
            })?;
        if status.is_success() {
            return Ok(bytes);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| MatrixError::Unavailable {
                context: "下载头像".into(),
                message: format!("错误响应不是 JSON：{error}"),
            })?;
        Err(MatrixError::Rejected {
            context: "下载头像".into(),
            status: status.as_u16(),
            errcode: value["errcode"].as_str().unwrap_or_default().into(),
            message: value["error"].as_str().unwrap_or_default().into(),
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: RequestBody<'_>,
        context: &str,
    ) -> Result<Value, MatrixError> {
        let url = format!("{}{path}", self.base);
        let (status, value) = self
            .http
            .request_json(method, &url, token, &[], body, TIMEOUT)
            .await
            .map_err(|e| MatrixError::Unavailable {
                context: context.to_owned(),
                message: e.to_string(),
            })?;
        if status.is_success() {
            return Ok(value);
        }
        Err(MatrixError::Rejected {
            context: context.to_owned(),
            status: status.as_u16(),
            errcode: value["errcode"].as_str().unwrap_or_default().to_owned(),
            message: value["error"].as_str().unwrap_or_default().to_owned(),
        })
    }

    /// 等 Tuwunel 能回答客户端接口（升级后首次启动可能在迁移数据库）。
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), MatrixError> {
        let deadline = Instant::now() + timeout;
        loop {
            let result = self
                .json(
                    Method::GET,
                    "/_matrix/client/versions",
                    None,
                    None,
                    "查询版本",
                )
                .await;
            match result {
                Ok(_) => return Ok(()),
                Err(e) if Instant::now() >= deadline => return Err(e),
                Err(_) => sleep(Duration::from_millis(500)).await,
            }
        }
    }

    /// 用核心签的 JWT 登录，新建一台设备。不申请刷新令牌：这个访问令牌不过期，吊销设备时才失效。
    pub async fn login(&self, jwt: &str, device_name: &str) -> Result<Login, MatrixError> {
        let body = json!({
            "type": JWT_LOGIN_TYPE,
            "token": jwt,
            "initial_device_display_name": device_name,
        });
        let value = self
            .json(
                Method::POST,
                "/_matrix/client/v3/login",
                None,
                Some(&body),
                "JWT 登录",
            )
            .await?;
        serde_json::from_value(value)
            .map_err(|e| MatrixError::Invalid(format!("登录响应不对：{e}")))
    }

    /// 令牌属于谁；令牌已失效时返回 `None`。
    pub async fn whoami(&self, token: &str) -> Result<Option<String>, MatrixError> {
        let path = "/_matrix/client/v3/account/whoami";
        match self
            .json(Method::GET, path, Some(token), None, "查询令牌")
            .await
        {
            Ok(value) => Ok(value["user_id"].as_str().map(str::to_owned)),
            Err(MatrixError::Rejected { status, .. })
                if status == StatusCode::UNAUTHORIZED.as_u16() =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

/// 路径和查询参数里的一段：Matrix 的 ID 含 `!`、`#`、`:` 等字符，按 RFC 3986 编码保留字符以外的一切。
pub fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
