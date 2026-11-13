//! 转发头的取舍（ADR-0061 第 3、10 节）：使用方自带的 `X-Wonton-Plugin`、`X-Wonton-User`、
//! `X-Wonton-Chain`、上限头全部丢弃，`X-Wonton-Bot` 的使用方值也丢——发给提供方的目标 Bot 由核心
//! 注入、覆盖使用方的值（ADR-0053）。核心重新注入算出的身份、链、上限和提供方令牌。

use hyper::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use wonton_protocol::{
    ChainLimits, WONTON_BOT_HEADER, WONTON_CHAIN_HEADER, WONTON_MAX_DEPTH_HEADER,
    WONTON_MAX_ROUNDS_HEADER, WONTON_MAX_TOKENS_HEADER, WONTON_PLUGIN_HEADER, WONTON_USER_HEADER,
};

/// 提供方握手（对核心转发这条连接的升级请求）要带的头。
pub(super) fn provider_handshake(
    consumer: &str,
    user: &str,
    chain: &str,
    target: &wonton_protocol::BotRef,
    limits: ChainLimits,
    token: &str,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        WONTON_PLUGIN_HEADER,
        HeaderValue::from_str(consumer).expect("插件 ID 是合法的 HTTP 请求头"),
    );
    headers.insert(
        WONTON_USER_HEADER,
        HeaderValue::from_str(user).expect("Matrix 用户 ID 是合法的 HTTP 请求头"),
    );
    headers.insert(
        WONTON_BOT_HEADER,
        HeaderValue::from_str(&format!("{}#{}", target.plugin_id, target.bot))
            .expect("插件 ID 和 Bot 名是合法的 HTTP 请求头"),
    );
    headers.insert(
        WONTON_CHAIN_HEADER,
        HeaderValue::from_str(chain).expect("链 ID 是十六进制，是合法的 HTTP 请求头"),
    );
    headers.insert(
        WONTON_MAX_DEPTH_HEADER,
        HeaderValue::from_str(&limits.max_depth.to_string()).expect("数字是合法的 HTTP 请求头"),
    );
    headers.insert(
        WONTON_MAX_ROUNDS_HEADER,
        HeaderValue::from_str(&limits.max_rounds.to_string()).expect("数字是合法的 HTTP 请求头"),
    );
    headers.insert(
        WONTON_MAX_TOKENS_HEADER,
        HeaderValue::from_str(&limits.max_tokens.to_string()).expect("数字是合法的 HTTP 请求头"),
    );
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
        .expect("令牌是十六进制，是合法的 HTTP 请求头");
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    headers
}
