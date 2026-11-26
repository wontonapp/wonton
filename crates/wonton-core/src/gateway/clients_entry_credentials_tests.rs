//! 凭据入口（`auth: credentials`）：签发前一直 409，签发后按插件命令验证用户名和值；
//! 没有 verify 命令的清单只认自己签出去的值。

use super::*;
use base64::Engine as _;

const CREDENTIALS_APP: &str = r#"
services:
  app:
    image: example/credentials:1
x-wonton:
  spec: '0.4'
  id: io.wonton.test.credentials
  version: 1.0.0
  kind: app
  title: { zh: 凭据入口, en: Credentials entry }
  description:
    zh: 凭据入口行为测试用
    en: For credentials entry behaviour tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          username: "{email}"
          issue: [pw, issue, "{name}"]
          verify: [pw, verify, "{name}"]
          revoke: [pw, revoke, "{name}"]
          ends_sessions: false
"#;

/// 只认一个值：verify 拿标准输入比对，其余操作成功。
struct Verifier {
    value: Arc<Mutex<String>>,
    issued: bool,
}

impl crate::clients::credentials::CredentialExec for Mutex<Verifier> {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        argv: &'a [String],
        stdin: Option<&'a [u8]>,
        _timeout: Duration,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<crate::clients::credentials::ExecOutput, crate::clients::credentials::ExecError>,
    > {
        Box::pin(async move {
            let mut verifier = self.lock().unwrap();
            verifier.issued = true;
            let supplied = String::from_utf8_lossy(stdin.unwrap_or_default())
                .trim()
                .to_owned();
            let code = if argv[1] == "verify" && supplied != *verifier.value.lock().unwrap() {
                1
            } else {
                0
            };
            Ok(crate::clients::credentials::ExecOutput {
                exit_code: code,
                stdout: String::new(),
                stderr: String::new(),
            })
        })
    }
}

fn verifier() -> Arc<Mutex<Verifier>> {
    Arc::new(Mutex::new(Verifier {
        value: Arc::new(Mutex::new(String::new())),
        issued: false,
    }))
}

#[tokio::test]
async fn credentials_entry_stays_closed_until_first_issue() {
    let home = Home::start().await;
    home.register(CREDENTIALS_APP, "credapp", false, true);
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.credentials").unwrap();
    let verifier = verifier();
    home.manager.set_credential_exec(verifier.clone());
    let public = home.host("credapp");
    let closed = home.get(&public, "/", &entry_public()).await;
    assert_eq!(closed.status, StatusCode::CONFLICT);

    home.manager.issue_new(&id, "手机", None).await.unwrap();
    assert!(verifier.lock().unwrap().issued);
    // 没发过密码时不能开；发过以后没带 Authorization 的请求（应用自己的登录）原样转给应用。
    let opened = home.get(&public, "/", &entry_public()).await;
    assert_eq!(opened.status, StatusCode::OK, "{}", opened.text());
    assert_eq!(home.upstream_hits()[0].value("authorization"), None);
}

#[tokio::test]
async fn credentials_pass_through_missing_and_bearer() {
    let home = Home::start().await;
    home.register(CREDENTIALS_APP, "credapp", false, true);
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.credentials").unwrap();
    let verifier = verifier();
    home.manager.set_credential_exec(verifier.clone());
    let issued = home.manager.issue_new(&id, "手机", None).await.unwrap();
    let crate::clients::pending::Issue::Done(issued) = issued else {
        panic!("插件在跑，应该发出去");
    };
    *verifier.lock().unwrap().value.lock().unwrap() = issued.value.clone();

    let public = home.host("credapp");
    let anonymous = home
        .get(&public, "/.well-known/immich", &entry_public())
        .await;
    assert_eq!(anonymous.status, StatusCode::OK, "{}", anonymous.text());
    assert_eq!(home.upstream_hits()[0].value("authorization"), None);

    let bearer = "Bearer eyJhbGciOi.immich.session";
    let token = home
        .get(
            &public,
            "/api/assets",
            &[("x-wonton-entry", "public"), ("authorization", bearer)],
        )
        .await;
    assert_eq!(token.status, StatusCode::OK, "{}", token.text());
    assert_eq!(home.upstream_hits()[1].value("authorization"), Some(bearer));

    let wrong = base64::engine::general_purpose::STANDARD.encode("me@example.com:wrong");
    let denied = home
        .get(
            &public,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("authorization", &format!("Basic {wrong}")),
            ],
        )
        .await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    assert_eq!(home.upstream_hits().len(), 2);
}

#[tokio::test]
async fn credentials_accept_any_nonempty_username() {
    let home = Home::start().await;
    home.register(CREDENTIALS_APP, "credapp", false, true);
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.credentials").unwrap();
    let verifier = verifier();
    home.manager.set_credential_exec(verifier.clone());
    let issued = home.manager.issue_new(&id, "手机", None).await.unwrap();
    let crate::clients::pending::Issue::Done(issued) = issued else {
        panic!("插件在跑，应该发出去");
    };
    *verifier.lock().unwrap().value.lock().unwrap() = issued.value.clone();

    let public = home.host("credapp");
    let email = base64::engine::general_purpose::STANDARD
        .encode(format!("me@example.com:{}", issued.value));
    let ok = home
        .get(
            &public,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("authorization", &format!("Basic {email}")),
            ],
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(home.upstream_hits().len(), 1);
    assert_eq!(
        home.upstream_hits()[0].value("authorization"),
        Some(format!("Basic {email}").as_str())
    );

    let wrong = base64::engine::general_purpose::STANDARD.encode("me@example.com:wrong");
    let denied = home
        .get(
            &public,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("authorization", &format!("Basic {wrong}")),
            ],
        )
        .await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    assert_eq!(home.upstream_hits().len(), 1);
}

const NO_VERIFY_APP: &str = r#"
services:
  app:
    image: example/credentials:1
x-wonton:
  spec: '0.4'
  id: io.wonton.test.noverify
  version: 1.0.0
  kind: app
  title: { zh: 无验证凭据, en: No-verify credentials }
  description:
    zh: 没有 verify 命令的凭据入口
    en: Credentials entry without verify
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          issue: [pw, issue, "{name}"]
          revoke: [pw, revoke, "{name}"]
          ends_sessions: false
"#;

#[tokio::test]
async fn credentials_without_verify_trusts_issued_value() {
    let home = Home::start().await;
    home.register(NO_VERIFY_APP, "noverify", false, true);
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.noverify").unwrap();
    let crate::clients::pending::Issue::Done(issued) =
        home.manager.issue_new(&id, "手机", None).await.unwrap()
    else {
        panic!("插件在跑，应该发出去");
    };
    let host = home.host("noverify");
    let ok = base64::engine::general_purpose::STANDARD.encode(format!("owner:{}", issued.value));
    let reply = home
        .get(
            &host,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("authorization", &format!("Basic {ok}")),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let wrong = base64::engine::general_purpose::STANDARD.encode("owner:nope");
    let denied = home
        .get(
            &host,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("authorization", &format!("Basic {wrong}")),
            ],
        )
        .await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
}
