//! 好友经分享连接打开代登录插件：`{user}` 是固定在 friends.json 里的本机用户名，`{name}` 是显示名，
//! `{role}` 是 `friend`。浏览器带来的同名请求头和 Cookie 一律换掉。

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Router;
use axum::extract::Request;
use axum::response::IntoResponse;
use hyper::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use wonton_protocol::{ShareGrantUpdate, ShareRecipient};

use super::Fixture;
use crate::gateway::tests::TOKEN;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginPackage, PluginStatus, proxy_auth::ProxyAuth};

const GAME: &str = "name: game
services:
  web:
    image: example/game
x-wonton:
  spec: '0.13'
  id: app.game
  version: 0.1.0
  kind: app
  title: {zh: 游戏, en: Game}
  description: {zh: 多人游戏, en: Multiplayer game}
  webui:
    service: web
    port: 80
    accounts: per_user
    sso: proxy
    proxy:
      headers:
        X-Remote-Role: '{role}'
        X-Remote-User: '{user}'
        X-Remote-Name: '{name}'
        X-Remote-Email: '{email}'
      cookies: {game_web: '{secret:web_key}'}
  secrets:
    web_key: {generate: {bytes: 32}}
  permissions: []
";

const GAME_HOST: &str = "app-game.wonton.localhost";
const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// 假插件：把收到的请求头原样回显。
async fn upstream() -> SocketAddr {
    let router = Router::new().fallback(|req: Request| async move {
        let received: Vec<(String, String)> = req
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into(),
                )
            })
            .collect();
        axum::Json(json!({"headers": received})).into_response()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

async fn register(f: &Fixture) {
    let langs = Languages::from_header(Some("zh-Hans"));
    let manifest = PluginPackage::parse(GAME, &langs).unwrap().manifest;
    let generated = BTreeMap::from([("web_key".to_owned(), KEY.to_owned())]);
    let auth = ProxyAuth::resolve(&manifest, &generated, &langs)
        .unwrap()
        .unwrap();
    let id = manifest.id.clone();
    f.h.manager.register_for_test(
        manifest,
        PluginStatus::Running,
        Some(upstream().await.to_string()),
    );
    f.h.manager.proxy_for_test(&id, auth);
    f.sharing
        .shares
        .create(
            &ShareGrantUpdate {
                plugin_id: id.as_str().to_owned(),
                recipient: ShareRecipient::Friend {
                    user_id: f.peer.user_id.clone(),
                },
                expires_at_ms: None,
                acknowledge_risk: true,
            },
            &f.h.manager.share_offer(&id),
        )
        .unwrap();
}

fn received(body: &str, name: &str) -> Vec<String> {
    let body: Value = serde_json::from_str(body).unwrap();
    body["headers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pair| pair[0].as_str().unwrap().eq_ignore_ascii_case(name))
        .map(|pair| pair[1].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn the_friend_is_signed_in_with_its_fixed_name_and_the_user_role() {
    let f = Fixture::new().await;
    register(&f).await;
    let friend = f.sharing.friends.get(&f.peer.user_id).unwrap();
    let headers = [
        ("x-remote-role", "owner"),
        ("X-Remote-User", "smokeowner"),
        ("x-remote-name", "Mallory"),
        ("cookie", "game_web=forged; theme=dark"),
    ];
    let reply = f.get_with(GAME_HOST, "/", &headers).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json().to_string();
    assert_eq!(received(&body, "x-remote-role"), vec!["user".to_owned()]);
    assert_eq!(
        received(&body, "x-remote-user"),
        vec![friend.username.clone()]
    );
    assert_eq!(received(&body, "x-remote-name"), vec!["Alice".to_owned()]);
    // 内部邮箱按固定的本机用户名派生，只认人不收信。
    assert_eq!(
        received(&body, "x-remote-email"),
        vec![format!("{}@wonton.internal", friend.username)]
    );
    // 浏览器带来的同名 Cookie 换掉，其余 Cookie 留着。
    assert_eq!(
        received(&body, "cookie"),
        [format!("theme=dark; game_web={KEY}")]
    );
    // 分享连接不认主人的会话 Cookie；浏览器带来的 Bearer 只是原样转给插件，不当身份。
    assert_eq!(
        received(&body, "authorization"),
        vec![format!("Bearer {TOKEN}")]
    );
}

#[tokio::test]
async fn a_friend_is_rendered_as_a_regular_user() {
    let langs = Languages::from_header(Some("en"));
    let manifest = PluginPackage::parse(GAME, &langs).unwrap().manifest;
    let generated = BTreeMap::from([("web_key".to_owned(), KEY.to_owned())]);
    let auth = ProxyAuth::resolve(&manifest, &generated, &langs)
        .unwrap()
        .unwrap();
    let owner = crate::owner::Owner {
        username: "smokeowner".into(),
        display_name: "Owner".into(),
        email: Some("owner@wonton.internal".into()),
    };
    let headers = auth.headers(&owner, false).unwrap();
    let value = headers
        .iter()
        .find(|(name, _)| name.as_str() == "x-remote-role")
        .unwrap();
    assert_eq!(value.1, "user");
}
