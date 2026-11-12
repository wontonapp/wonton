//! 地址只表示 registry 的 authority，不接受 URL 或隐含的认证信息。

pub(crate) fn validate(address: &str) -> Result<(), String> {
    let invalid = || {
        format!("镜像加速地址 {address:?} 不合法：只填主机名和可选端口，不带协议、路径或用户名密码")
    };
    let (host, port) = match address.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (address, None),
    };
    if host.is_empty() || host.len() > 253 {
        return Err(invalid());
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            || !label
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(invalid());
        }
    }
    if let Some(port) = port
        && (port.is_empty()
            || !port.bytes().all(|c| c.is_ascii_digit())
            || port.parse::<u16>().ok().filter(|port| *port > 0).is_none())
    {
        return Err(invalid());
    }
    Ok(())
}

pub(in crate::plugin) fn choices(value: serde_json::Value) -> Result<Vec<String>, String> {
    let choices: Vec<String> = serde_json::from_value(value).map_err(|e| e.to_string())?;
    for address in &choices {
        validate(address)?;
    }
    Ok(choices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_addresses_are_bare_hosts_with_optional_ports() {
        for valid in [
            "mirror.example",
            "mirror.example:5443",
            "127.0.0.1:5000",
            "localhost",
        ] {
            assert!(validate(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "https://mirror.example",
            "mirror.example/",
            "user@mirror.example",
            "mirror.example?x",
            "mirror.example#x",
            " mirror.example",
            "mirror.example:0",
            "mirror.example:65536",
            "mirror.example:",
            "mirror.example:abc",
            "a..b",
            "-mirror.example",
            "mirror_.example",
        ] {
            assert!(validate(invalid).is_err(), "{invalid}");
        }
    }
}
