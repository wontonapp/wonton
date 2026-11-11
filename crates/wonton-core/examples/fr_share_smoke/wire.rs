use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use wonton_protocol::MatrixOpenIdToken;
use wonton_tunnel::sharing::{ShareClient, ShareStream};

pub async fn token(server: &str, user: &str) -> Result<MatrixOpenIdToken> {
    let http = super::owner::https()?;
    let key = std::fs::read_to_string(super::owner::state()?.join("matrix.key"))?;
    let issuer = wonton_core::matrix::issuer::Issuer::from_pem(&key).map_err(anyhow::Error::msg)?;
    let login: Value = http
        .post(format!("https://{server}/_matrix/client/v3/login"))
        .json(&json!({"type":"org.matrix.login.jwt", "token":issuer.login_token(user)}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id = login["user_id"].as_str().context("login user id")?;
    ensure!(
        id == format!("@{user}:{server}"),
        "homeserver returned unexpected login identity"
    );
    let access = login["access_token"]
        .as_str()
        .context("login access token")?;
    let body: Value = http
        .post(format!(
            "https://{server}/_matrix/client/v3/user/{id}/openid/request_token"
        ))
        .bearer_auth(access)
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    // Exercise the homeserver's real token endpoint and userinfo before presenting it to Wonton.
    let sub: Value = http
        .get(format!(
            "https://{server}/_matrix/federation/v1/openid/userinfo"
        ))
        .query(&[(
            "access_token",
            body["access_token"]
                .as_str()
                .context("OpenID access token")?,
        )])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        sub["sub"] == id,
        "real homeserver userinfo identity differs"
    );
    Ok(serde_json::from_value(body)?)
}

pub type Socket = BufReader<ShareStream>;

pub async fn websocket(client: &ShareClient, url: &str) -> Result<(Socket, Value)> {
    let url = url::Url::parse(url)?;
    let host = url.host_str().context("plugin host")?;
    let port = url.port_or_known_default().context("plugin port")?;
    let mut stream = client.open_forward().await?;
    stream.write_all(format!("GET /ws HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nX-Remote-User: forged\r\nX-Remote-Role: owner\r\n\r\n").as_bytes()).await?;
    let mut socket = BufReader::new(stream);
    let mut line = String::new();
    socket.read_line(&mut line).await?;
    ensure!(
        line.starts_with("HTTP/1.1 101"),
        "WebSocket upgrade refused: {line}"
    );
    let mut accept = false;
    let mut bytes = line.len();
    loop {
        line.clear();
        ensure!(
            socket.read_line(&mut line).await? != 0,
            "EOF during upgrade"
        );
        bytes += line.len();
        ensure!(bytes <= 16384, "oversize upgrade headers");
        if line == "\r\n" {
            break;
        }
        if line
            .to_ascii_lowercase()
            .starts_with("sec-websocket-accept:")
        {
            accept = line.trim().ends_with("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
        }
    }
    ensure!(accept, "invalid WebSocket accept");
    let identity = serde_json::from_slice(&frame(&mut socket).await?)?;
    socket
        .get_mut()
        .write_all(&[
            0x81,
            0x84,
            1,
            2,
            3,
            4,
            b'p' ^ 1,
            b'i' ^ 2,
            b'n' ^ 3,
            b'g' ^ 4,
        ])
        .await?;
    ensure!(
        frame(&mut socket).await? == b"ping",
        "WebSocket echo differs"
    );
    Ok((socket, identity))
}

async fn frame(socket: &mut Socket) -> Result<Vec<u8>> {
    let first = socket.read_u8().await?;
    let second = socket.read_u8().await?;
    ensure!(first == 0x81 && second & 0x80 == 0, "invalid fixture frame");
    let length = match second {
        126 => usize::from(socket.read_u16().await?),
        127 => anyhow::bail!("oversize fixture frame"),
        n => usize::from(n),
    };
    let mut bytes = vec![0; length];
    socket.read_exact(&mut bytes).await?;
    Ok(bytes)
}

pub async fn reset(mut socket: Socket) -> Result<()> {
    let result = tokio::time::timeout(Duration::from_secs(3), socket.read_u8()).await;
    ensure!(
        matches!(result, Ok(Err(_))),
        "revoke did not reset the established WebSocket within 3 seconds"
    );
    Ok(())
}
