//! 签名的中继列表（ADR-0048）：取 `relays.json` 和旁边的 `.minisig`，用内置公钥验 minisign 签名。
//!
//! 取不到、验不过都只是「这次不用签名列表」，错误写进设置页的提示；内置默认和主人配置照常生效。
//! 列表里不放令牌：签名的列表是公开的，令牌属于部署方，写在主人配置里。

use std::time::Duration;

use serde::Deserialize;
use wonton_tunnel::relays::RelayTable;

use super::{OwnerEntry, OwnerFile};

/// 签名列表和签名的上限。几台中继的 JSON 远小于它，挡住异常上游撑爆内存。
const MAX_BYTES: usize = 256 * 1024;

/// 一次取列表和签名的总时限。启动时等太久不如先起来，下次再说。
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
struct SignedFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    relays: Vec<OwnerEntry>,
}

/// 取签名列表并验签，返回这一层表。`proxy` 是出网代理（`WONTON_EGRESS_PROXY`）。
pub(super) async fn fetch(
    url: &str,
    public_key_b64: &str,
    proxy: Option<&str>,
) -> Result<RelayTable, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("wonton-core/", env!("CARGO_PKG_VERSION")));
    if let Some(proxy) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy).map_err(|error| format!("出网代理地址不合法：{error}"))?,
        );
    }
    let client = builder
        .build()
        .map_err(|error| format!("建 HTTP 客户端：{error}"))?;
    let data = get(&client, url).await?;
    let signature_url = signature_url(url)?;
    let signature_text = get(&client, &signature_url).await?;
    let signature_text = String::from_utf8(signature_text)
        .map_err(|_| "签名文件不是 UTF-8 的 minisign 签名".to_owned())?;
    verified(&data, &signature_text, public_key_b64)
}

/// 验签并解析：`data` 是文件内容，`signature_text` 是 `.minisig` 文件内容。
fn verified(data: &[u8], signature_text: &str, public_key_b64: &str) -> Result<RelayTable, String> {
    let public_key = minisign_verify::PublicKey::from_base64(public_key_b64.trim())
        .map_err(|error| format!("WONTON_RELAY_LIST_PUBKEY 不是合法的 minisign 公钥：{error}"))?;
    let signature = minisign_verify::Signature::decode(signature_text)
        .map_err(|error| format!("签名文件不是合法的 minisign 签名：{error}"))?;
    public_key
        .verify(data, &signature, false)
        .map_err(|error| format!("签名验不过：{error}"))?;
    let file: SignedFile = serde_json::from_slice(data)
        .map_err(|error| format!("签名列表不是合法的 JSON：{error}"))?;
    if file.version > 1 {
        return Err(format!("签名列表的版本 {} 比这个核心新", file.version));
    }
    OwnerFile {
        version: 1,
        n0: true,
        relays: file.relays,
        remove: Vec::new(),
    }
    .to_layer()
    .map(|layer| layer.relays)
    .map_err(|reason| format!("签名列表不合法：{reason}"))
}

/// 签名和列表同目录、同查询：`…/relays.json?x=1` 的签名是 `…/relays.json.minisig?x=1`，
/// 不在 query 后面直接拼 `.minisig`。
fn signature_url(url: &str) -> Result<String, String> {
    let mut parsed =
        reqwest::Url::parse(url).map_err(|error| format!("列表地址不合法：{error}"))?;
    parsed.set_path(&format!("{}.minisig", parsed.path()));
    Ok(parsed.into())
}

async fn get(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let shown = redact_url(url);
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("取 {shown}：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("取 {shown}：HTTP {status}"));
    }
    // 边读边卡：Content-Length 可以撒谎或根本没有（chunked），不能先收完再判。
    if let Some(length) = response.content_length()
        && length > MAX_BYTES as u64
    {
        return Err(format!("{shown} 超过 {MAX_BYTES} 字节"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读 {shown}：{error}"))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_BYTES {
            return Err(format!("{shown} 超过 {MAX_BYTES} 字节"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// 报错时打的地址：userinfo 里的令牌换成 `<redacted>`。
fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    // userinfo 只可能在第一个 `/`、`?`、`#` 之前；query 里的 `@` 不算。
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = match authority.split_once('@') {
        Some((_, host)) => format!("<redacted>@{host}"),
        None => authority.to_owned(),
    };
    // query 里常常是令牌：键保留，值一律打码。
    let tail = match tail.split_once('?') {
        Some((head, query)) => {
            let keys: Vec<_> = query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(|pair| pair.split('=').next().unwrap_or(pair))
                .map(|key| format!("{key}=<redacted>"))
                .collect();
            format!("{head}?{}", keys.join("&"))
        }
        None => tail.to_owned(),
    };
    format!("{scheme}://{authority}{tail}")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use minisign::KeyPair;

    use super::*;

    fn signed(data: &[u8]) -> (String, String) {
        let keypair = KeyPair::generate_unencrypted_keypair().unwrap();
        let signature = minisign::sign(
            Some(&keypair.pk),
            &keypair.sk,
            Cursor::new(data),
            None,
            None,
        )
        .unwrap();
        let public_key = keypair.pk.to_box().unwrap().to_string();
        let public_key_b64 = public_key.lines().nth(1).unwrap().to_owned();
        (signature.to_string(), public_key_b64)
    }

    fn list_json(url: &str) -> Vec<u8> {
        format!(r#"{{"version":1,"relays":[{{"url":"{url}","tier":0}}]}}"#).into_bytes()
    }

    #[test]
    fn a_valid_signature_parses_the_list() {
        let data = list_json("https://relay.example.org");
        let (signature, public_key) = signed(&data);
        let table = verified(&data, &signature, &public_key).unwrap();
        assert_eq!(table.relays()[0].url.as_str(), "https://relay.example.org/");
    }

    #[test]
    fn a_changed_body_fails_verification() {
        let data = list_json("https://relay.example.org");
        let (signature, public_key) = signed(&data);
        let tampered = list_json("https://evil.example.org");
        let error = verified(&tampered, &signature, &public_key).unwrap_err();
        assert!(
            error.contains("验不过") || error.contains("签名"),
            "{error}"
        );
    }

    #[test]
    fn the_list_url_token_is_redacted_in_errors() {
        assert_eq!(
            redact_url("https://tok@relay.example.org/relays.json"),
            "https://<redacted>@relay.example.org/relays.json"
        );
        assert_eq!(
            redact_url("https://relay.example.org"),
            "https://relay.example.org"
        );
    }

    #[test]
    fn the_signature_url_gets_a_sig_path_before_the_query() {
        assert_eq!(
            signature_url("https://relay.example.org/a/relays.json?token=x").unwrap(),
            "https://relay.example.org/a/relays.json.minisig?token=x"
        );
    }

    #[test]
    fn a_query_token_is_redacted_in_errors() {
        let shown = redact_url("https://relay.example.org/relays.json?token=x&v=1");
        assert_eq!(
            shown,
            "https://relay.example.org/relays.json?token=<redacted>&v=<redacted>"
        );
        assert!(!shown.contains("=x"));
    }

    #[test]
    fn an_at_sign_in_the_query_is_not_mistaken_for_userinfo() {
        assert_eq!(
            redact_url("https://relay.example.org/p?email=a@b&v=1"),
            "https://relay.example.org/p?email=<redacted>&v=<redacted>"
        );
    }

    #[test]
    fn a_version_newer_than_this_core_is_reported() {
        let data = br#"{"version":2,"relays":[]}"#;
        let (signature, public_key) = signed(data);
        let error = verified(data, &signature, &public_key).unwrap_err();
        assert!(error.contains("版本 2"), "{error}");
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn a_chunked_body_without_content_length_is_cut_at_the_limit() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}/relays.json", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let chunk = vec![b'x'; 40_000];
            for _ in 0..8 {
                socket
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(&chunk).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
        });
        let error = get(&reqwest::Client::new(), &url).await.unwrap_err();
        assert!(error.contains("超过"), "{error}");
    }
}
