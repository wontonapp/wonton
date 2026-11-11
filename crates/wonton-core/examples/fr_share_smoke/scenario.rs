use super::{
    owner::{self, Owner},
    wire,
};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;
use wonton_protocol::{GatewayRefusal, MatrixOpenIdToken, SecretString, ShareIdentityFailure};
use wonton_tunnel::sharing::{ShareClient, ShareError};

pub async fn run() -> Result<()> {
    let a = Owner::open("owner").await?;
    let b = Owner::open("friend").await?;
    owner::pair(&a, &b).await?;
    install(&a).await?;
    let grants = grant(&a).await?;
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
        wire::token("friend.test", "bob"),
        wire::token("external.test", "carol")
    )?;
    let (bob, carol) = tokio::try_join!(
        ShareClient::connect(endpoint.clone(), target.clone(), bob),
        ShareClient::connect(endpoint.clone(), target, carol)
    )?;
    ensure!(
        bob.user_id() == "@bob:friend.test" && carol.user_id() == "@carol:external.test",
        "wrong share identities"
    );
    // 配对过的好友走 iroh 联邦，标准发现这条路不走；前门只有联邦请求会写这个标记。
    ensure!(
        owner::state()?.join("paired-userinfo").exists(),
        "paired friend did not verify over the iroh federation route"
    );
    multiplayer(&a, &bob, &carol, &grants).await?;
    bob.close();
    carol.close();
    endpoint.close().await;
    a.close().await;
    b.close().await;
    println!(
        "PASS: two Wonton cores, paired iroh identity, external Tuwunel identity, strict discovery, distinct multiplayer usernames, both WebSockets reset on revoke"
    );
    Ok(())
}

async fn install(owner: &Owner) -> Result<()> {
    let mut tar = tar::Builder::new(Vec::new());
    // 包根就是 compose.yaml 所在的那一层，不要再套一层目录。
    tar.append_dir_all(".", "/fixture/multiplayer")?;
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
    let tar = bytes;
    let response = owner
        .http
        .post(format!("http://{}:8080/_wonton/api/plugins", owner.domain))
        .header("Content-Type", "application/x-tar")
        // 安装要建 Bot、起容器并等健康，比普通接口久。
        .timeout(Duration::from_secs(300))
        .body(tar.clone())
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

async fn grant(owner: &Owner) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for user in ["@bob:friend.test", "@carol:external.test"] {
        let grant = owner
            .api(
                Method::POST,
                "/_wonton/api/shares",
                Some(json!({
                    "plugin_id":"app.share-smoke", "recipient":{"type":"friend","user_id":user},
                    "expires_at_ms":null, "acknowledge_risk":false
                })),
            )
            .await?;
        ids.push(grant["id"].as_str().context("grant id")?.to_owned());
    }
    Ok(ids)
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

async fn multiplayer(
    owner: &Owner,
    bob: &ShareClient,
    carol: &ShareClient,
    grants: &[String],
) -> Result<()> {
    let bob_app = bob
        .initial_shares()
        .plugins
        .iter()
        .find(|p| p.plugin_id == "app.share-smoke")
        .context("Bob app missing")?;
    let carol_app = carol
        .initial_shares()
        .plugins
        .iter()
        .find(|p| p.plugin_id == "app.share-smoke")
        .context("Carol app missing")?;
    let ((bob_ws, bob_result), (carol_ws, carol_result)) = tokio::try_join!(
        wire::websocket(bob, &bob_app.web_url),
        wire::websocket(carol, &carol_app.web_url)
    )?;
    let friends = owner.api(Method::GET, "/_wonton/api/friends", None).await?;
    verify_identity(&friends, &bob_result, "@bob:friend.test")?;
    verify_identity(&friends, &carol_result, "@carol:external.test")?;
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
    for id in grants {
        owner
            .api(Method::DELETE, &format!("/_wonton/api/shares/{id}"), None)
            .await?;
    }
    tokio::try_join!(wire::reset(bob_ws), wire::reset(carol_ws))?;
    for client in [bob, carol] {
        ensure!(
            client.list().await?.plugins.is_empty(),
            "revoked app remains listed"
        );
    }
    Ok(())
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
    ensure!(
        result["identity"]["role"] == "friend",
        "proxy role spoofed or missing"
    );
    ensure!(
        result["identity"]["name"] == friend["display_name"],
        "proxy display name differs"
    );
    Ok(())
}
