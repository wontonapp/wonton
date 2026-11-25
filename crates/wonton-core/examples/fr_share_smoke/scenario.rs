use super::{
    owner::{self, Owner},
    wire,
};
use anyhow::{Context, Result, ensure};
use iroh::{Endpoint, EndpointAddr};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;
use wonton_protocol::manifest::PluginId;
use wonton_protocol::{
    GatewayRefusal, MatrixOpenIdToken, SecretString, ShareIdentityFailure, SharedPlugin,
};
use wonton_tunnel::sharing::{ShareClient, ShareError};

const APP: &str = "app.share-smoke";
const OTHER: &str = "app.share-other";
const BOB: &str = "@bob:friend.test";
const CAROL: &str = "@carol:external.test";

pub async fn run() -> Result<()> {
    let a = Owner::open("owner").await?;
    let b = Owner::open("friend").await?;
    owner::pair(&a, &b).await?;
    install(&a, "multiplayer").await?;
    install(&a, "second").await?;
    open_to_friends(&a).await?;
    let grants = grant(&a, APP, &[BOB, CAROL]).await?;
    let target = wonton_client::discover_share_source("owner.test")
        .await
        .map_err(|e| anyhow::anyhow!("client discovery failed: {e:?}"))?
        .context("owner discovery missing")?;
    ensure!(
        wonton_client::discover_share_source("bad-cert.test")
            .await
            .is_err(),
        "actual client accepted invalid well-known certificate"
    );
    let endpoint = owner::endpoint().await?;
    negative(&endpoint, &target).await?;
    let (bob, carol) = tokio::try_join!(
        connect(&endpoint, &target, "friend.test", "bob"),
        connect(&endpoint, &target, "external.test", "carol")
    )?;
    ensure!(
        bob.user_id() == BOB && carol.user_id() == CAROL,
        "wrong share identities"
    );
    // 配对过的好友走 iroh 联邦，标准发现这条路不走；前门只有联邦请求会写这个标记。
    ensure!(
        owner::state()?.join("paired-userinfo").exists(),
        "paired friend did not verify over the iroh federation route"
    );
    multiplayer(&a, &endpoint, &target, &bob, &carol, &grants).await?;
    bob.close();
    carol.close();
    endpoint.close().await;
    a.close().await;
    b.close().await;
    println!(
        "PASS: two Wonton cores, paired iroh identity, external Tuwunel identity, strict discovery, distinct multiplayer usernames, \
         no admin/Matrix/file/ungranted-plugin reach, both WebSockets reset on revoke and friend removal, grant recovery after re-add, \
         federation block add/remove"
    );
    Ok(())
}

async fn connect(
    endpoint: &Endpoint,
    target: &EndpointAddr,
    server: &str,
    user: &str,
) -> Result<ShareClient> {
    let token = wire::token(server, user).await?;
    Ok(ShareClient::connect(endpoint.clone(), target.clone(), token).await?)
}

/// 分享连接只允许开页面。这里换成普通 HTTP 调别处，按拒绝类别断言。
async fn unreachable(
    client: &ShareClient,
    method: &str,
    host: &str,
    path: &str,
    label: &str,
) -> Result<()> {
    let (status, headers, body) = wire::probe(client, method, host, path).await?;
    ensure!(status == 403, "{label}: expected 403, got {status}: {body}");
    ensure!(
        wire::header(&headers, "x-wonton-error") == Some("share_required"),
        "{label}: wrong refusal: {headers:?}"
    );
    Ok(())
}

/// 授权过的插件页面照常打开；url 形如 http://host:8080/。
async fn page_ok(client: &ShareClient, url: &str, marker: &str) -> Result<()> {
    let url = url::Url::parse(url)?;
    let host = url.host_str().context("plugin host")?;
    let path = match url.path() {
        "" => "/",
        path => path,
    };
    let (status, _, body) = wire::probe(client, "GET", host, path).await?;
    ensure!(status == 200, "page {url} gave {status}: {body}");
    ensure!(body.contains(marker), "page {url} body lacks {marker}");
    Ok(())
}

async fn install(owner: &Owner, directory: &str) -> Result<()> {
    let mut tar = tar::Builder::new(Vec::new());
    // 包根就是 compose.yaml 所在的那一层，不要再套一层目录。
    tar.append_dir_all(".", format!("/fixture/{directory}"))?;
    let bytes = tar.into_inner()?;
    ensure!(
        tar::Archive::new(bytes.as_slice())
            .entries()?
            .filter_map(Result::ok)
            .any(|entry| entry
                .path()
                .is_ok_and(|path| path.ends_with("docker-compose.yml"))),
        "packed plugin archive has no docker-compose.yml: {bytes:?}"
    );
    let response = owner
        .http
        .post(format!("http://{}:8080/_wonton/api/plugins", owner.domain))
        .header("Content-Type", "application/x-tar")
        // 安装要建 Bot、起容器并等健康，比普通接口久。
        .timeout(Duration::from_secs(300))
        .body(bytes)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("install request failed: {error:?}"))?;
    let status = response.status();
    let body = response.text().await?;
    ensure!(
        status.as_u16() == 201,
        "plugin installation {status}: {body}"
    );
    Ok(())
}
/// 分享按好友的角色算（ADR-0088）：默认的 friend 角色一个新应用都不开，先把两个插件开给它。
async fn open_to_friends(owner: &Owner) -> Result<()> {
    for plugin in [APP, OTHER] {
        owner
            .api(
                Method::PUT,
                &format!("/_wonton/api/plugins/{plugin}/audience"),
                Some(json!({"role_ids":["friend"]})),
            )
            .await?;
    }
    Ok(())
}

async fn grant(owner: &Owner, plugin: &str, users: &[&str]) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for user in users {
        let grant = owner
            .api(
                Method::POST,
                "/_wonton/api/shares",
                Some(json!({
                    "plugin_id":plugin, "recipient":{"type":"friend","user_id":user},
                    "expires_at_ms":null, "acknowledge_risk":false
                })),
            )
            .await?;
        ids.push(grant["id"].as_str().context("grant id")?.to_owned());
    }
    Ok(ids)
}

async fn revoke(owner: &Owner, grants: &[String]) -> Result<()> {
    for id in grants {
        owner
            .api(Method::DELETE, &format!("/_wonton/api/shares/{id}"), None)
            .await?;
    }
    Ok(())
}

async fn negative(endpoint: &iroh::Endpoint, target: &iroh::EndpointAddr) -> Result<()> {
    let forged = MatrixOpenIdToken {
        access_token: SecretString::new("forged-smoke-token"),
        matrix_server_name: "external.test".into(),
        expires_in: 3600,
    };
    refused(endpoint, target, forged, None).await?;
    let mut mismatch = wire::token("external.test", "carol").await?;
    mismatch.matrix_server_name = "mismatch.test".into();
    refused(
        endpoint,
        target,
        mismatch,
        Some(ShareIdentityFailure::ServerNameMismatch),
    )
    .await?;
    let mut bad_cert = wire::token("external.test", "carol").await?;
    bad_cert.matrix_server_name = "bad-cert.test".into();
    refused(
        endpoint,
        target,
        bad_cert,
        Some(ShareIdentityFailure::DiscoveryFailed),
    )
    .await
}

async fn refused(
    endpoint: &iroh::Endpoint,
    target: &iroh::EndpointAddr,
    token: MatrixOpenIdToken,
    identity: Option<ShareIdentityFailure>,
) -> Result<()> {
    match ShareClient::connect(endpoint.clone(), target.clone(), token).await {
        Err(ShareError::Refused(reason)) => {
            ensure!(
                reason.refusal == GatewayRefusal::ShareIdentity,
                "identity check returned wrong refusal"
            );
            if let Some(identity) = identity {
                ensure!(
                    reason.identity == Some(identity),
                    "wrong identity category: {reason:?}"
                );
            }
            Ok(())
        }
        Err(error) => anyhow::bail!("identity negative failed outside authentication: {error}"),
        Ok(client) => {
            client.close();
            anyhow::bail!("invalid identity authenticated")
        }
    }
}

/// 两个已登录的访客和各自能看到的多人插件。
struct Pair<'a> {
    bob: &'a ShareClient,
    carol: &'a ShareClient,
    bob_app: SharedPlugin,
    carol_app: SharedPlugin,
}

async fn multiplayer(
    owner: &Owner,
    endpoint: &Endpoint,
    target: &EndpointAddr,
    bob: &ShareClient,
    carol: &ShareClient,
    grants: &[String],
) -> Result<()> {
    let pair = Pair {
        bob,
        carol,
        bob_app: shared(bob, APP)?.clone(),
        carol_app: shared(carol, APP)?.clone(),
    };
    isolation(&pair).await?;
    let (bob_ws, carol_ws) = sessions(owner, &pair).await?;
    let (other_grant, carol_again, carol_ws) =
        revoke_and_restore(owner, endpoint, target, &pair, grants, bob_ws, carol_ws).await?;
    friend_lifecycle(
        owner,
        endpoint,
        target,
        &carol_again,
        carol_ws,
        &pair.carol_app,
    )
    .await?;
    carol_again.close();
    federation_blocks(owner).await?;
    revoke(owner, &other_grant).await
}

fn plugin_host(plugin: &str) -> Result<String> {
    Ok(format!("{}.owner.test", PluginId::parse(plugin)?.slug()))
}

/// 授权过的页面能开；未授权的插件、管理接口、Matrix、文件都进不去。
async fn isolation(pair: &Pair<'_>) -> Result<()> {
    page_ok(pair.bob, &pair.bob_app.web_url, "multiplayer").await?;
    page_ok(pair.carol, &pair.carol_app.web_url, "multiplayer").await?;
    let other = plugin_host(OTHER)?;
    unreachable(pair.bob, "GET", &other, "/", "plugin without a grant").await?;
    unreachable(
        pair.carol,
        "GET",
        &other,
        "/",
        "plugin without a grant (carol)",
    )
    .await?;
    unreachable(
        pair.bob,
        "GET",
        "owner.test",
        "/_wonton/api/peers",
        "owner admin API",
    )
    .await?;
    unreachable(
        pair.bob,
        "GET",
        "matrix.owner.test",
        "/_matrix/client/v3/versions",
        "owner Matrix",
    )
    .await?;
    unreachable(
        pair.bob,
        "GET",
        "owner.test",
        "/_wonton/files/none/content",
        "owner files",
    )
    .await
}

/// 两人各开一条 WebSocket 进同一个多人房间：用户名来自持久化好友记录、互不相同，房间里有两个人。
async fn sessions(owner: &Owner, pair: &Pair<'_>) -> Result<(wire::Socket, wire::Socket)> {
    let ((bob_ws, bob_result), (carol_ws, carol_result)) = tokio::try_join!(
        wire::websocket(pair.bob, &pair.bob_app.web_url),
        wire::websocket(pair.carol, &pair.carol_app.web_url)
    )?;
    let friends = owner.api(Method::GET, "/_wonton/api/friends", None).await?;
    verify_identity(&friends, &bob_result, BOB)?;
    verify_identity(&friends, &carol_result, CAROL)?;
    ensure!(
        bob_result["identity"]["user"] != carol_result["identity"]["user"],
        "friends share a plugin username"
    );
    let players = bob_result["players"]
        .as_array()
        .context("Bob players")?
        .len()
        .max(
            carol_result["players"]
                .as_array()
                .context("Carol players")?
                .len(),
        );
    ensure!(
        players == 2,
        "concurrent players did not reach one shared multiplayer room"
    );
    Ok((bob_ws, carol_ws))
}

/// 第二个插件单独授权、收回、再授权：授权当场生效，收回时两条 WebSocket 在 3 秒内断，重新授权能恢复。
async fn revoke_and_restore(
    owner: &Owner,
    endpoint: &Endpoint,
    target: &EndpointAddr,
    pair: &Pair<'_>,
    grants: &[String],
    bob_ws: wire::Socket,
    carol_ws: wire::Socket,
) -> Result<(Vec<String>, ShareClient, wire::Socket)> {
    let other = plugin_host(OTHER)?;
    let other_grant = grant(owner, OTHER, &[BOB]).await?;
    page_ok(pair.bob, &format!("http://{other}:8080/"), "second").await?;
    unreachable(
        pair.carol,
        "GET",
        &other,
        "/",
        "plugin granted to someone else",
    )
    .await?;

    revoke(owner, grants).await?;
    wire::reset(bob_ws, "bob revoke").await?;
    wire::reset(carol_ws, "carol revoke").await?;
    let bob_shares = pair.bob.list().await?;
    ensure!(
        bob_shares.plugins.len() == 1 && bob_shares.plugins[0].plugin_id == OTHER,
        "Bob still lists revoked apps: {bob_shares:?}"
    );
    ensure!(
        pair.carol.list().await?.plugins.is_empty(),
        "revoked app remains listed"
    );

    // 收回后重新授权：新连接页面和 WebSocket 都能恢复。
    grant(owner, APP, &[CAROL]).await?;
    let carol_again = connect(endpoint, target, "external.test", "carol").await?;
    page_ok(&carol_again, &pair.carol_app.web_url, "multiplayer").await?;
    let (carol_ws, _) = wire::websocket(&carol_again, &pair.carol_app.web_url).await?;
    Ok((other_grant, carol_again, carol_ws))
}

/// 删好友连带收回：连接当场断、分享被拒；重新同意后同一个人的本机用户名不变，重新授权恢复。
async fn friend_lifecycle(
    owner: &Owner,
    endpoint: &Endpoint,
    target: &EndpointAddr,
    carol_again: &ShareClient,
    carol_ws: wire::Socket,
    carol_app: &SharedPlugin,
) -> Result<()> {
    let old_username = friend_username(CAROL)?;
    owner
        .api(
            Method::DELETE,
            &format!("/_wonton/api/friends/{CAROL}"),
            None,
        )
        .await?;
    wire::reset(carol_ws, "carol friend delete").await?;
    ensure!(
        matches!(carol_again.list().await, Err(ShareError::Refused(_))),
        "removed friend still lists shares"
    );

    owner
        .api(
            Method::PUT,
            &format!("/_wonton/api/friends/{CAROL}"),
            Some(json!({"display_name":"Carol"})),
        )
        .await?;
    ensure!(
        friend_username(CAROL)? == old_username,
        "friend plugin username changed across removal"
    );
    let third = grant(owner, APP, &[CAROL]).await?;
    let carol_third = connect(endpoint, target, "external.test", "carol").await?;
    page_ok(&carol_third, &carol_app.web_url, "multiplayer").await?;
    let (carol_ws, carol_result) = wire::websocket(&carol_third, &carol_app.web_url).await?;
    let friends = owner.api(Method::GET, "/_wonton/api/friends", None).await?;
    verify_identity(&friends, &carol_result, CAROL)?;
    revoke(owner, &third).await?;
    wire::reset(carol_ws, "carol revoke after re-add").await?;
    carol_third.close();
    Ok(())
}

/// 整台服务器拉黑和撤销。
async fn federation_blocks(owner: &Owner) -> Result<()> {
    owner
        .api(
            Method::POST,
            "/_wonton/api/federation-blocks/external.test",
            None,
        )
        .await?;
    let friends = owner.api(Method::GET, "/_wonton/api/friends", None).await?;
    ensure!(
        friends["blocked_servers"]
            .as_array()
            .context("blocked_servers")?
            .iter()
            .any(|server| server == "external.test"),
        "blocked server missing: {friends:?}"
    );
    owner
        .api(
            Method::DELETE,
            "/_wonton/api/federation-blocks/external.test",
            None,
        )
        .await?;
    let friends = owner.api(Method::GET, "/_wonton/api/friends", None).await?;
    ensure!(
        !friends["blocked_servers"]
            .as_array()
            .context("blocked_servers")?
            .iter()
            .any(|server| server == "external.test"),
        "blocked server survived unblock: {friends:?}"
    );
    Ok(())
}

fn shared<'a>(client: &'a ShareClient, plugin: &str) -> Result<&'a SharedPlugin> {
    client
        .initial_shares()
        .plugins
        .iter()
        .find(|row| row.plugin_id == plugin)
        .with_context(|| format!("{plugin} missing from shares"))
}

fn friend_username(user: &str) -> Result<String> {
    let store =
        wonton_core::friends::Friends::open(owner::state()?.join("owner/core/friends.json"))?;
    store
        .get(user)
        .map(|friend| friend.username)
        .context("friend missing")
}

fn verify_identity(friends: &Value, result: &Value, user: &str) -> Result<()> {
    let friend = friends["friends"]
        .as_array()
        .context("friends rows")?
        .iter()
        .find(|friend| friend["user_id"] == user)
        .context("friend missing")?;
    let store =
        wonton_core::friends::Friends::open(owner::state()?.join("owner/core/friends.json"))?;
    let record = store.get(user).context("persisted friend missing")?;
    ensure!(
        result["identity"]["user"] == record.username,
        "plugin username differs from persisted friend"
    );
    // ADR-0088 第 8 节：`{role}` 只有 admin 和 user，不是 ADR-0051 草稿里的 friend。
    ensure!(
        result["identity"]["role"] == "user",
        "proxy role spoofed or missing"
    );
    ensure!(
        result["identity"]["name"] == friend["display_name"],
        "proxy display name differs"
    );
    Ok(())
}
