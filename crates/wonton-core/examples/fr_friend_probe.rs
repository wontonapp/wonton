//! 以好友的身份打开主人分享的插件，打印插件拿到的身份。用法见 deploy/dev/core/friend-probe.sh：
//! `setup` 配对、装 /package 里的插件并分享给 Bob；`visit` 以 Bob 的身份经分享隧道打开插件网页，跟着重定向
//! 走完插件自己的登录（`sso: oidc` 时就是核心的 OIDC 授权），再从 Hydra 管理接口读出这次授权给插件的声明。
#[path = "fr_share_smoke/owner.rs"]
mod owner;
// 只用到取 OpenID token 的那一个函数，WebSocket 的部分给分享冒烟用。
#[allow(dead_code)]
#[path = "fr_share_smoke/wire.rs"]
mod wire;

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Method, StatusCode, header};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path, time::Duration};
use wonton_tunnel::sharing::ShareClient;

use owner::Owner;

const BOB: &str = "@bob:friend.test";
const PACKAGE: &str = "/package";
const HYDRA_ADMIN: &str = "http://hydra.wonton.internal:4445";

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("setup") => setup().await,
        Some("visit") => visit().await,
        _ => bail!("usage: fr_friend_probe setup | visit"),
    }
}

async fn setup() -> Result<()> {
    let a = Owner::open("owner").await?;
    let b = Owner::open("friend").await?;
    owner::pair(&a, &b).await?;
    let id = install(&a).await?;
    a.api(
        Method::POST,
        "/_wonton/api/shares",
        Some(json!({
            "plugin_id": id, "recipient": {"type": "friend", "user_id": BOB},
            "expires_at_ms": null, "acknowledge_risk": true
        })),
    )
    .await?;
    std::fs::write(owner::state()?.join("friend-plugin"), &id)?;
    println!("已安装 {id} 并分享给 {BOB}");
    a.close().await;
    b.close().await;
    Ok(())
}

/// 和 `dev.sh install --yes` 一样：先预览，再授予预览列出的全部权限安装。
async fn install(owner: &Owner) -> Result<String> {
    let path = Path::new(PACKAGE);
    let (body, kind) = if path.is_dir() {
        let mut tar = tar::Builder::new(Vec::new());
        tar.append_dir_all(".", path)?;
        (tar.into_inner()?, "application/x-tar")
    } else {
        (std::fs::read(path)?, "application/yaml")
    };
    let base = format!("http://{}:8080/_wonton/api/plugins", owner.domain);
    let preview = owner
        .http
        .post(format!("{base}/preview"))
        .header(header::CONTENT_TYPE, kind)
        .body(body.clone())
        .send()
        .await?;
    let status = preview.status();
    let preview: Value = preview.json().await?;
    ensure!(status.is_success(), "插件预览失败 {status}: {preview}");
    if let Some(blocked) = preview["blocked"].as_str() {
        bail!("这台核心上装不了：{blocked}");
    }
    let id = preview["id"]
        .as_str()
        .context("预览里没有插件 ID")?
        .to_owned();
    let grants: Vec<&str> = preview["permissions"]
        .as_array()
        .context("预览里没有权限列表")?
        .iter()
        .filter_map(|p| p["permission"].as_str())
        .collect();
    println!("安装 {id}，授予权限：{}", grants.join(",").as_str());
    let response = owner
        .http
        .post(&base)
        .query(&[("grant", grants.join(","))])
        .header(header::CONTENT_TYPE, kind)
        // 安装要拉起容器并等健康，比普通接口久。
        .timeout(Duration::from_secs(600))
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    ensure!(
        status == StatusCode::CREATED,
        "插件安装失败 {status}: {text}"
    );
    Ok(id)
}

async fn visit() -> Result<()> {
    let id = std::fs::read_to_string(owner::state()?.join("friend-plugin"))?;
    let target = wonton_client::discover_share_source("owner.test")
        .await
        .map_err(|e| anyhow::anyhow!("分享来源发现失败：{e:?}"))?
        .context("owner.test 没有发布分享来源")?;
    let endpoint = owner::endpoint().await?;
    let token = wire::token("friend.test", "bob").await?;
    let bob = ShareClient::connect(endpoint.clone(), target, token)
        .await
        .map_err(|e| anyhow::anyhow!("Bob 连不上分享隧道：{e}"))?;
    let web_url = bob
        .initial_shares()
        .plugins
        .iter()
        .find(|p| p.plugin_id == id)
        .with_context(|| format!("分享给 Bob 的插件里没有 {id}"))?
        .web_url
        .clone();
    serve_local(bob.clone()).await?;
    let friend =
        wonton_core::friends::Friends::open(owner::state()?.join("owner/core/friends.json"))?
            .get(BOB)
            .context("friends.json 里没有 Bob")?;
    println!(
        "好友 {BOB}：本机用户名 {}，显示名 {}",
        friend.username,
        friend.display_name.as_deref().unwrap_or("")
    );
    browse(&web_url).await?;
    print_oidc(&friend.username, &id).await?;
    bob.close();
    endpoint.close().await;
    Ok(())
}

/// 好友的 App 在本机开的代理：每条 TCP 连接原样转进分享隧道，网关按 Host 选路由。
async fn serve_local(bob: ShareClient) -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let bob = bob.clone();
            tokio::spawn(async move {
                if let Err(error) = bob.forward_raw_tcp(tcp).await {
                    eprintln!("分享隧道转发失败：{error}");
                }
            });
        }
    });
    Ok(())
}

/// 像浏览器一样跟着重定向走，带上每台主机自己的 Cookie；只认 owner.test 下的主机，别的地址好友到不了。
async fn browse(start: &str) -> Result<()> {
    let local: std::net::SocketAddr = "127.0.0.1:8080".parse()?;
    let start = url::Url::parse(start)?;
    let host = start.host_str().context("插件网页地址没有主机名")?;
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .resolve(host, local)
        .resolve("auth.owner.test", local)
        .resolve("owner.test", local)
        .build()?;
    let mut jar: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut url = start;
    println!("以 Bob 的身份打开：");
    for _ in 0..30 {
        let host = url.host_str().context("重定向地址没有主机名")?.to_owned();
        ensure!(
            host == "owner.test" || host.ends_with(".owner.test"),
            "插件把浏览器送到了 {url}，好友只能经分享隧道打开主人服务器上的地址"
        );
        let cookies = jar.entry(host).or_default();
        let mut request = http.get(url.clone()).header(header::ACCEPT, "text/html");
        if !cookies.is_empty() {
            let line: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            request = request.header(header::COOKIE, line.join("; "));
        }
        let response = request.send().await?;
        let status = response.status();
        for value in response.headers().get_all(header::SET_COOKIE) {
            let pair = value.to_str()?.split(';').next().unwrap_or_default();
            if let Some((name, value)) = pair.split_once('=') {
                if value.is_empty() {
                    cookies.remove(name.trim());
                } else {
                    cookies.insert(name.trim().to_owned(), value.to_owned());
                }
            }
        }
        let location = response.headers().get(header::LOCATION).cloned();
        println!("  {} {}", status.as_u16(), redact(&url));
        let Some(location) = location.filter(|_| status.is_redirection()) else {
            ensure!(
                status.is_success(),
                "插件网页返回 {status}: {}",
                response.text().await?
            );
            return Ok(());
        };
        url = url.join(location.to_str()?)?;
    }
    bail!("重定向超过 30 次")
}

/// 查询串里的授权码、challenge、verifier 都是一次性凭据，不打出来。
fn redact(url: &url::Url) -> String {
    let mut shown = url.clone();
    shown.set_query(None);
    match url.query() {
        Some(_) => format!("{shown}?…"),
        None => shown.to_string(),
    }
}

/// 核心接受授权时把同一份声明同时写进 ID 令牌和 userinfo（crates/wonton-core/src/oidc.rs 的 `id_token_claims`）。
async fn print_oidc(username: &str, id: &str) -> Result<()> {
    let sessions: Value = reqwest::Client::builder()
        .no_proxy()
        .build()?
        .get(format!("{HYDRA_ADMIN}/admin/oauth2/auth/sessions/consent"))
        .query(&[("subject", username)])
        .send()
        .await
        .context("连不上 Hydra 的管理接口")?
        .error_for_status()?
        .json()
        .await?;
    let ours: Vec<&Value> = sessions
        .as_array()
        .context("Hydra 的授权记录不是数组")?
        .iter()
        .filter(|s| s["consent_request"]["client"]["client_id"] == id)
        .collect();
    let Some(last) = ours.last() else {
        println!("OIDC：{id} 没有以 Bob 的身份走过 OIDC 授权（sso 不是 oidc，或插件没跳去登录）");
        return Ok(());
    };
    println!("OIDC 授予的 scope：{}", last["grant_scope"]);
    // sub 由 Hydra 按登录时接受的 subject 填，不在核心写的声明里。
    println!("OIDC sub：{}", last["consent_request"]["subject"]);
    println!(
        "OIDC ID 令牌声明（也是 userinfo）：\n{}",
        serde_json::to_string_pretty(&last["session"]["id_token"])?
    );
    Ok(())
}
