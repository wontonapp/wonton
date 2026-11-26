//! HTTP Basic/Bearer parsing for gateway-authenticated client entries.

use base64::Engine as _;
use hyper::HeaderMap;

#[derive(Debug, PartialEq, Eq)]
pub enum Attempt {
    Missing,
    Password(String),
    Invalid,
}

pub fn parse_authorization(headers: &HeaderMap, username: &str) -> Attempt {
    parse(headers, Some(username))
}

/// `credentials` 用：清单里的用户名只是提示给主人，客户端可能发别的（如自己的邮箱），
/// 所以只要求 Basic 用户名非空，真正校验交给应用里的 `verify`。
pub fn parse_password(headers: &HeaderMap) -> Attempt {
    parse(headers, None)
}

/// `credentials` 的入口验证分两类：`Basic` 里的密码由核心交给应用里的 `verify` 判断；
/// 没带 `Authorization`（应用自己的登录页、`/.well-known` 探测）和 `Bearer`（应用登录后自己的会话令牌）
/// 原样转给应用，由应用验证。`NonBasic` 也照转，不认识别的方案不拦。
#[derive(Debug, PartialEq, Eq)]
pub enum CredentialsAttempt {
    Missing,
    NonBasic,
    Password(String),
    Invalid,
}

pub fn parse_credentials(headers: &HeaderMap) -> CredentialsAttempt {
    let Some(value) = headers.get(hyper::header::AUTHORIZATION) else {
        return CredentialsAttempt::Missing;
    };
    let Ok(value) = value.to_str() else {
        return CredentialsAttempt::Invalid;
    };
    let Some((scheme, encoded)) = value.split_once(char::is_whitespace) else {
        return CredentialsAttempt::Invalid;
    };
    if scheme.eq_ignore_ascii_case("bearer") {
        return if encoded.trim().is_empty() {
            CredentialsAttempt::Invalid
        } else {
            CredentialsAttempt::NonBasic
        };
    }
    if !scheme_is_basic(scheme) {
        return CredentialsAttempt::NonBasic;
    }
    match decode_basic(encoded, None) {
        Ok(password) => CredentialsAttempt::Password(password),
        Err(()) => CredentialsAttempt::Invalid,
    }
}

fn scheme_is_basic(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("basic")
}

fn decode_basic(encoded: &str, username: Option<&str>) -> Result<String, ()> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| ())?;
    let decoded = String::from_utf8(decoded).map_err(|_| ())?;
    let (user, password) = decoded.split_once(':').ok_or(())?;
    if user.is_empty() || password.is_empty() || username.is_some_and(|wanted| wanted != user) {
        return Err(());
    }
    Ok(password.to_owned())
}

fn parse(headers: &HeaderMap, username: Option<&str>) -> Attempt {
    let Some(value) = headers.get(hyper::header::AUTHORIZATION) else {
        return Attempt::Missing;
    };
    let Ok(value) = value.to_str() else {
        return Attempt::Invalid;
    };
    let Some((scheme, encoded)) = value.split_once(char::is_whitespace) else {
        return Attempt::Invalid;
    };
    if scheme.eq_ignore_ascii_case("bearer") {
        let value = encoded.trim();
        if value.is_empty() {
            return Attempt::Invalid;
        }
        return Attempt::Password(value.to_owned());
    }
    if !scheme_is_basic(scheme) {
        return Attempt::Invalid;
    }
    match decode_basic(encoded, username) {
        Ok(password) => Attempt::Password(password),
        Err(()) => Attempt::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_missing_and_invalid_authorization() {
        let mut headers = HeaderMap::new();
        assert_eq!(parse_authorization(&headers, "owner"), Attempt::Missing);
        let basic = base64::engine::general_purpose::STANDARD.encode("owner:secret");
        headers.insert("authorization", format!("Basic {basic}").parse().unwrap());
        assert_eq!(
            parse_authorization(&headers, "owner"),
            Attempt::Password("secret".into())
        );
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        assert_eq!(
            parse_authorization(&headers, "owner"),
            Attempt::Password("secret".into())
        );
        headers.insert("authorization", "Basic invalid".parse().unwrap());
        assert_eq!(parse_authorization(&headers, "owner"), Attempt::Invalid);

        // credentials：用户名只看非空，邮箱也放行。
        let email = base64::engine::general_purpose::STANDARD.encode("me@example.com:secret");
        headers.insert("authorization", format!("Basic {email}").parse().unwrap());
        assert_eq!(parse_password(&headers), Attempt::Password("secret".into()));
        let empty = base64::engine::general_purpose::STANDARD.encode(":secret");
        headers.insert("authorization", format!("Basic {empty}").parse().unwrap());
        assert_eq!(parse_password(&headers), Attempt::Invalid);
    }

    #[test]
    fn credentials_only_verifies_basic() {
        let mut headers = HeaderMap::new();
        assert_eq!(parse_credentials(&headers), CredentialsAttempt::Missing);
        headers.insert("authorization", "Bearer eye-em-token".parse().unwrap());
        assert_eq!(parse_credentials(&headers), CredentialsAttempt::NonBasic);
        headers.insert("authorization", "Digest realm=x".parse().unwrap());
        assert_eq!(parse_credentials(&headers), CredentialsAttempt::NonBasic);
        headers.insert("authorization", "Bearer ".parse().unwrap());
        assert_eq!(parse_credentials(&headers), CredentialsAttempt::Invalid);
        let email = base64::engine::general_purpose::STANDARD.encode("me@example.com:secret");
        headers.insert("authorization", format!("Basic {email}").parse().unwrap());
        assert_eq!(
            parse_credentials(&headers),
            CredentialsAttempt::Password("secret".into())
        );
        headers.insert("authorization", "Basic not-base64".parse().unwrap());
        assert_eq!(parse_credentials(&headers), CredentialsAttempt::Invalid);
    }
}
