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
    if !scheme.eq_ignore_ascii_case("basic") {
        return Attempt::Invalid;
    }
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return Attempt::Invalid;
    };
    let Ok(decoded) = String::from_utf8(decoded) else {
        return Attempt::Invalid;
    };
    let Some((user, password)) = decoded.split_once(':') else {
        return Attempt::Invalid;
    };
    if user.is_empty() || password.is_empty() || username.is_some_and(|wanted| wanted != user) {
        return Attempt::Invalid;
    }
    Attempt::Password(password.to_owned())
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
}
