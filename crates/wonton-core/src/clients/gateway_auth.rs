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
    if user != username || password.is_empty() {
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
    }
}
