//! 经隧道向核心要 Matrix 登录令牌（ADR-0009 的“接口：App 登录和使用 Matrix”）。
//!
//! 请求走本地代理，核心凭隧道身份认出这台已配对设备。令牌只能用一次、两分钟内有效，拿到就登录。

use std::time::Duration;

use serde::Deserialize;
use url::Url;
use wonton_protocol::{ChatFailure, ProxySettings};

/// 核心签发的登录令牌，字段同核心的 `LoginToken`（核心有契约测试锁住字段名）。
#[derive(Clone, Deserialize)]
pub(crate) struct LoginToken {
    pub(crate) homeserver_url: String,
    pub(crate) user_id: String,
    pub(crate) login_type: String,
    pub(crate) token: String,
}

/// 核心的错误响应：`{"error": {"kind": …, "message": …}}`。
#[derive(Deserialize)]
pub(super) struct ErrorBody {
    pub(super) error: ErrorDetail,
}

#[derive(Deserialize)]
pub(super) struct ErrorDetail {
    pub(super) kind: String,
    pub(super) message: String,
}

const JWT_LOGIN: &str = "org.matrix.login.jwt";

/// 要令牌的整个请求（连上、发出、读完回应）最多等这么久，和 matrix-sdk 默认的请求超时一样。隧道或本地代理
/// 收下请求却一直不回时，登录按“传输不可用”失败，回到前台或隧道重连时再试，不会一直停在“正在登录”。
pub(crate) const LOGIN_TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

/// 本地代理的地址，带上本次启动的会话凭据，给 HTTP 客户端用。
pub(crate) fn proxy_url(proxy: &ProxySettings) -> String {
    format!(
        "http://{}:{}@{}:{}",
        proxy.username, proxy.password, proxy.host, proxy.port
    )
}

/// `POST <核心地址>/_wonton/matrix/login-token`，整个请求最多等 `timeout`（见 [`LOGIN_TOKEN_TIMEOUT`]）。
pub(crate) async fn login_token(
    core_url: &Url,
    proxy: &ProxySettings,
    timeout: Duration,
) -> Result<LoginToken, (ChatFailure, String)> {
    let url = core_url
        .join("/_wonton/matrix/login-token")
        .expect("固定的路径总能拼上");
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| {
            (
                ChatFailure::InvalidInput,
                format!("本地代理地址不合法：{e}"),
            )
        })?)
        .timeout(timeout)
        .build()
        .map_err(|e| (ChatFailure::Upstream, format!("建 HTTP 客户端失败：{e}")))?;
    let transport = |what: &str, e: reqwest::Error| {
        let reason = if e.is_timeout() {
            format!("{} 秒内没有回应", timeout.as_secs_f32())
        } else {
            e.to_string()
        };
        (
            ChatFailure::TransportUnavailable,
            format!("{what}：{reason}"),
        )
    };
    let response = client
        .post(url)
        .send()
        .await
        .map_err(|e| transport("向核心要 Matrix 登录令牌失败", e))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|e| transport("读核心的回应失败", e))?;
    if !status.is_success() {
        return Err(core_error(status.as_u16(), &body));
    }
    let token: LoginToken = serde_json::from_slice(&body).map_err(|e| {
        (
            ChatFailure::Upstream,
            format!("核心给的登录令牌格式不对：{e}"),
        )
    })?;
    if token.login_type != JWT_LOGIN {
        return Err((
            ChatFailure::Upstream,
            format!("核心给的登录方式 {} 不是 {JWT_LOGIN}", token.login_type),
        ));
    }
    Ok(token)
}

/// 按核心的错误码区分原因。
fn core_error(status: u16, body: &[u8]) -> (ChatFailure, String) {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let message = detail.as_ref().map_or_else(
        || format!("核心回了 {status}"),
        |detail| format!("{}（{}）", detail.message, detail.kind),
    );
    let reason = match (status, detail.as_ref().map(|detail| detail.kind.as_str())) {
        (_, Some("matrix_unavailable")) => ChatFailure::NotReady,
        (401 | 403, _) => ChatFailure::Unauthorized,
        (400, _) => ChatFailure::InvalidInput,
        // 本地代理连不上隧道时回 502，并在 x-wonton-tunnel-error 里写明原因。
        (502, _) => ChatFailure::TransportUnavailable,
        _ => ChatFailure::Upstream,
    };
    (reason, format!("向核心要 Matrix 登录令牌失败：{message}"))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use tokio::net::TcpListener;

    use super::*;

    /// 隧道卡住时，本地代理收下请求却一直不回：要令牌的请求到时间就按传输不可用失败，不会一直等下去。
    #[tokio::test]
    async fn a_proxy_that_never_answers_fails_as_transport_unavailable() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let silent = tokio::spawn(async move {
            // 连接都留着，不读也不回。
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let proxy = ProxySettings {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port,
            username: "phone".to_owned(),
            password: "secret".to_owned(),
            domain: "wonton.localhost".to_owned(),
        };
        let core = Url::parse("http://wonton.localhost").unwrap();

        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            login_token(&core, &proxy, Duration::from_millis(300)),
        )
        .await
        .expect("应在超时后失败，而不是一直等下去");
        let (reason, _) = outcome.err().expect("代理不回时应失败");
        assert_eq!(reason, ChatFailure::TransportUnavailable);
        silent.abort();
    }
}
