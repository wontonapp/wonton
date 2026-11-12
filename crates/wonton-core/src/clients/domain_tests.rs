use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use wonton_protocol::{CaaState, CredentialEntry, PutDomainRequest, SecretString};

use super::{DNS_ENV_FILE, Domain, DomainError, dotenv_names, dotenv_value};
use crate::plugin::docker::Engine;

fn domain(dir: &Path) -> std::sync::Arc<Domain> {
    Domain::new(
        dir.to_owned(),
        dir.join("data"),
        Engine::for_test(),
        "wonton-test-core".into(),
        crate::plugin::names::Instance::default(),
        None,
    )
}

fn settings(dir: &Path, server_name: Option<&str>) {
    let path = dir.join("entry/settings.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let settings = serde_json::json!({
        "domain": server_name.unwrap_or("private.example.test"),
        "service_url": null,
        "service_ca": null,
        "relay_ca": null,
        "registration": null,
        "acme_directory": "https://pebble.test:14000/dir",
        "acme_ca": null,
        "acme_resolvers": [],
        "acme_email": null,
        "acme_accept_terms": false,
        "acme_network": "bridge",
        "dns_provider": null,
        "dns_env_file": null,
        "external_certificate": false
    });
    fs::write(path, serde_json::to_vec(&settings).unwrap()).unwrap();
}

fn settings_with_provider(dir: &Path, env_file: &Path) {
    settings(dir, Some("abcdefgh.example.test"));
    let path = dir.join("entry/settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["acme_accept_terms"] = serde_json::json!(true);
    settings["dns_provider"] = serde_json::json!("httpreq");
    settings["dns_env_file"] = serde_json::json!(env_file);
    fs::write(path, serde_json::to_vec(&settings).unwrap()).unwrap();
}

fn request(suffix: Option<&str>) -> PutDomainRequest {
    PutDomainRequest {
        suffix: suffix.map(str::to_owned),
        provider: "httpreq".into(),
        credentials: vec![CredentialEntry {
            name: "HTTPREQ_ENDPOINT".into(),
            value: SecretString::new("https://relay.test/present"),
        }],
        accept_terms: true,
    }
}

fn certificate(dir: &Path) {
    let directory = dir.join("entry/certs");
    fs::create_dir_all(&directory).unwrap();
    let tls = rcgen::generate_simple_self_signed(vec!["example.test".to_owned()]).unwrap();
    fs::write(directory.join("site.pem"), tls.cert.pem()).unwrap();
    fs::write(directory.join("site.key"), tls.signing_key.serialize_pem()).unwrap();
}

#[tokio::test]
async fn put_domain_requires_terms() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let mut put = request(None);
    put.accept_terms = false;
    let error = domain(dir.path()).save(put).await.err().unwrap();
    assert!(matches!(error, DomainError::Invalid(_)), "{error}");
}

#[tokio::test]
async fn provider_name_charset_only() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let mut put = request(None);
    put.provider = "Cloudflare_DNS".into();
    assert!(matches!(
        domain(dir.path()).save(put.clone()).await,
        Err(DomainError::Invalid(_))
    ));
    // 核心不按服务商分支：不在任何预设里的代码照样收下。
    put.provider = "notapreset1".into();
    assert!(domain(dir.path()).save(put).await.is_ok());
}

/// 凭据只写不读：文件 0600，GET 只回“写过”，值不在设置页的状态里。
#[tokio::test]
async fn credentials_write_only_and_private() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let path = dir.path().join("entry/domain.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved["credentials"]["HTTPREQ_ENDPOINT"],
        "https://relay.test/present"
    );
    let view = clients.view().await.unwrap();
    assert!(view.credentials_set);
}

/// 自己的证书：设置收下也没人签，PUT 直接回冲突。
#[tokio::test]
async fn external_certificate_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let path = dir.path().join("entry/settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["external_certificate"] = serde_json::json!(true);
    fs::write(path, serde_json::to_vec(&settings).unwrap()).unwrap();
    assert!(matches!(
        domain(dir.path()).save(request(None)).await,
        Err(DomainError::Conflict(_))
    ));
}

#[tokio::test]
async fn suffix_fixed_when_server_name_set() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    let mut put = request(Some("other.example.test"));
    assert!(matches!(
        clients.save(put.clone()).await,
        Err(DomainError::Conflict(_))
    ));
    put.suffix = None;
    assert!(clients.save(put).await.is_ok());
    let view = clients.view().await.unwrap();
    assert_eq!(view.suffix.as_deref(), Some("abcdefgh.example.test"));
    assert!(!view.suffix_editable);
}

/// “不要名字”的安装没有 `entry/settings.json`：后缀能填、能改，改的时候回旧值给壳。
#[tokio::test]
async fn suffix_editable_in_none_mode() {
    let dir = tempfile::tempdir().unwrap();
    let clients = domain(dir.path());
    let saved = clients.save(request(Some("example.test"))).await.unwrap();
    assert_eq!(saved.previous_suffix, None);
    let saved = clients
        .save(request(Some("second.example.test")))
        .await
        .unwrap();
    assert!(saved.suffix_changed);
    assert_eq!(saved.previous_suffix.as_deref(), Some("example.test"));
    let view = clients.view().await.unwrap();
    assert!(view.suffix_editable);
    assert_eq!(view.suffix.as_deref(), Some("second.example.test"));
}

/// 签发失败：旧证书原样留着，状态带 lego 的错误。设置直接从装机环境来，不经 `save`
/// 排的那次后台签发，结果才确定。
#[tokio::test]
async fn issue_failure_keeps_previous_certificate_and_reports_error() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("provider.env");
    fs::write(&env_file, "HTTPREQ_ENDPOINT=https://relay.test/present\n").unwrap();
    settings_with_provider(dir.path(), &env_file);
    certificate(dir.path());
    let before = fs::read(dir.path().join("entry/certs/site.pem")).unwrap();
    let clients = domain(dir.path());
    // 假 Engine 连不上 Docker：签发死在起 lego 之前，错误要来自这一步。
    let error = clients.issue_now().await.unwrap_err();
    let text = format!("{error:#}").to_lowercase();
    assert!(
        text.contains("docker") || text.contains("lego") || text.contains("socket"),
        "{text}"
    );
    assert_eq!(
        fs::read(dir.path().join("entry/certs/site.pem")).unwrap(),
        before
    );
    match clients.view().await.unwrap().certificate {
        wonton_protocol::CertificateState::Renewing { error, .. } => {
            assert!(!error.is_empty())
        }
        other => panic!("状态应当是 renewing，实际 {other:?}"),
    }
}

/// 假解析结果只喂给分类函数：三种都按“对上账号 / 没有 / 对不上”进状态。
#[test]
fn caa_states() {
    use crate::entry_dns::{CaaIssue, caa_state};
    let account = "https://pebble.test/my-account/1";
    let bound = CaaIssue {
        tag: "issue".into(),
        issuer: Some("letsencrypt.org".into()),
        parameters: vec![
            ("accounturi".into(), account.into()),
            ("validationmethods".into(), "dns-01".into()),
        ],
    };
    assert_eq!(
        caa_state(&[bound], account, "letsencrypt.org"),
        CaaState::Bound
    );
    assert_eq!(
        caa_state(&[], account, "letsencrypt.org"),
        CaaState::Missing
    );
    let mismatch = CaaIssue {
        tag: "issue".into(),
        issuer: Some("letsencrypt.org".into()),
        parameters: vec![("accounturi".into(), "https://other.test/1".into())],
    };
    assert_eq!(
        caa_state(&[mismatch], account, "letsencrypt.org"),
        CaaState::Mismatch
    );
    // issuer 是别家：即使账号和方法都对也不算绑定（真实 RDATA 的版本在 entry_dns 的测试里）。
    let other_issuer = CaaIssue {
        tag: "issue".into(),
        issuer: Some("other-ca.example".into()),
        parameters: vec![
            ("accounturi".into(), account.into()),
            ("validationmethods".into(), "dns-01".into()),
        ],
    };
    assert_eq!(
        caa_state(&[other_issuer], account, "letsencrypt.org"),
        CaaState::Mismatch
    );
    let empty_issuer = CaaIssue {
        tag: "issue".into(),
        issuer: None,
        parameters: vec![
            ("accounturi".into(), account.into()),
            ("validationmethods".into(), "dns-01".into()),
        ],
    };
    assert_eq!(
        caa_state(&[empty_issuer], account, "letsencrypt.org"),
        CaaState::Mismatch
    );
}

/// 装机时的环境变量是初值：设置文件还不存在时，服务商、凭据和订户协议的勾选从这里来。
#[tokio::test]
async fn env_install_seeds_settings() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("provider.env");
    fs::write(&env_file, "DESEC_TOKEN='seeded'\n").unwrap();
    let path = dir.path().join("entry/settings.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let settings = serde_json::json!({
        "domain": "abcdefgh.example.test",
        "service_url": null,
        "service_ca": null,
        "relay_ca": null,
        "registration": null,
        "acme_directory": "https://pebble.test:14000/dir",
        "acme_ca": null,
        "acme_resolvers": [],
        "acme_email": null,
        "acme_accept_terms": true,
        "acme_network": "bridge",
        "dns_provider": "desec",
        "dns_env_file": env_file,
        "external_certificate": false
    });
    fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
    let view = domain(dir.path()).view().await.unwrap();
    assert_eq!(view.provider.as_deref(), Some("desec"));
    assert!(view.credentials_set);
    assert!(view.terms_accepted);
    assert!(!dir.path().join("entry/domain.json").exists());
}

#[test]
fn install_env_file_names_are_parsed_but_values_pass_through_untouched() {
    // 装机给的 dotenv 原样交给 lego：godotenv 的双引号、转义、展开语义归它，我们只认变量名
    // 做存在性检查。`export ` 前缀、双引号值、注释都不能把名字认错或把值改掉。
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dns.env");
    let original = b"\
CF_DNS_API_TOKEN=\"abc123\"\n\
export DESEC_TOKEN=abc\n\
export \t HTTPREQ_ENDPOINT='http://x'\n\
# comment\n\
\n\
EAAPI_KEY=\n\
WEIRD=value;not-params\n";
    fs::write(&path, original).unwrap();
    let names = dotenv_names(&path).unwrap();
    assert_eq!(
        names,
        [
            "CF_DNS_API_TOKEN",
            "DESEC_TOKEN",
            "HTTPREQ_ENDPOINT",
            "EAAPI_KEY",
            "WEIRD"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
    );
    assert_eq!(fs::read(&path).unwrap(), original, "文件必须原样保留");
    let empty = tempfile::tempdir().unwrap();
    let path = empty.path().join("empty.env");
    fs::write(&path, "# 只有注释\n").unwrap();
    assert!(dotenv_names(&path).is_err(), "一个变量名都没有要当场报错");
}

#[test]
fn dotenv_values_quote_metacharacters_for_godotenv() {
    // App 提交的凭据由我们写成 godotenv：单引号是字面量，`$`、反斜杠和空格都进单引号，
    // lego 实收的值和主人填的一致。
    assert_eq!(dotenv_value("plain-token_1"), "plain-token_1");
    assert_eq!(dotenv_value("x_./:+-y"), "x_./:+-y");
    assert_eq!(dotenv_value("user$TOKEN"), "'user$TOKEN'");
    assert_eq!(dotenv_value("a\\b$c"), "'a\\b$c'");
    assert_eq!(dotenv_value("with space"), "'with space'");
    assert_eq!(dotenv_value(""), "''");
}

/// 装机给的凭据文件原样传给 lego，不解析值：manager 挂的是原文件，App 没提交新凭据时一直用它。
#[tokio::test]
async fn install_env_file_is_passed_to_lego_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("provider.env");
    let original = b"export CF_DNS_API_TOKEN=\"tok$en\"\nCF_ZONE=\"a\\\\b\"\n";
    fs::write(&env_file, original).unwrap();
    settings_with_provider(dir.path(), &env_file);
    let clients = domain(dir.path());
    let store = clients.effective().unwrap();
    assert!(store.credentials.is_empty(), "值不进设置，原样留给 lego");
    assert_eq!(store.env_file.as_deref(), Some(env_file.as_path()));
    let view = clients.view().await.unwrap();
    assert!(view.credentials_set, "装机文件算凭据已备");
    let manager = clients.manager(&store).unwrap();
    assert_eq!(
        manager.config.dns_env_file.as_deref(),
        Some(env_file.as_path()),
        "lego 拿到的必须是原文件，不是重写出来的副本"
    );
    assert_eq!(manager.config.dns_provider.as_deref(), Some("httpreq"));
    assert_eq!(fs::read(&env_file).unwrap(), original, "原文件不能被动过");

    // 同服务商保存（凭据留空）不该要求重新提交凭据，装机文件继续有效。
    // 自己域名的安装 provider 必须是真实代码（家名安装才允许空），这里保持 httpreq。
    let mut put = request(Some("abcdefgh.example.test"));
    put.credentials.clear();
    assert!(clients.save(put).await.is_ok());
    let store = clients.effective().unwrap();
    assert_eq!(store.env_file.as_deref(), Some(env_file.as_path()));

    // 提交了新凭据：改用提交值，装机文件退场，lego 改挂我们写出的 dns.env。
    clients
        .save(request(Some("abcdefgh.example.test")))
        .await
        .unwrap();
    let store = clients.effective().unwrap();
    assert!(store.env_file.is_none());
    let manager = clients.manager(&store).unwrap();
    assert_eq!(
        manager.config.dns_env_file.as_deref(),
        Some(dir.path().join(DNS_ENV_FILE).as_path())
    );
}

#[tokio::test]
async fn credentials_accept_backslash_and_dollar_but_not_single_quote() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let mut put = request(None);
    put.credentials[0].value = SecretString::new("a\\b$c d");
    assert!(domain(dir.path()).save(put.clone()).await.is_ok());
    put.credentials[0].value = SecretString::new("it's");
    assert!(matches!(
        domain(dir.path()).save(put).await,
        Err(DomainError::Invalid(_))
    ));
}

fn registered_settings(dir: &Path) {
    settings(dir, Some("abcdefgh.example.test"));
    let path = dir.join("entry/settings.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let credentials = serde_json::json!({
        "username": "test", "password": "test", "subdomain": "test",
        "fulldomain": "test.auth.example.test"
    });
    value["registration"] = serde_json::json!({
        "home": "abcdefgh", "domain": "abcdefgh.example.test", "frp_token": "test",
        "acme_dns": credentials, "lan_acme_dns": credentials,
        "acme_dns_url": "https://auth.example.test",
        "relay": {"host": "relay.example.test", "port": 7000, "server_name": "relay.example.test"},
        "suspended": false
    });
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[tokio::test]
async fn registered_unchanged_save_preserves_prepared_issuance() {
    let dir = tempfile::tempdir().unwrap();
    registered_settings(dir.path());
    let clients = domain(dir.path());
    let mut put = request(Some("abcdefgh.example.test"));
    put.provider.clear();
    put.credentials.clear();
    let saved = clients.save(put).await.unwrap();
    assert!(!saved.provider_changed);
    let prepared = dir.path().join("entry/prepared.json");
    fs::write(&prepared, "in-progress").unwrap();
    let mut put = request(Some("abcdefgh.example.test"));
    put.provider.clear();
    put.credentials.clear();
    let saved = clients.save(put).await.unwrap();
    assert!(!saved.provider_changed);
    assert_eq!(fs::read_to_string(prepared).unwrap(), "in-progress");
}

#[tokio::test]
async fn provider_change_waits_for_inflight_issuance() {
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let serial = clients.issuance.lock().await;
    let prepared = dir.path().join("entry/prepared.json");
    fs::write(&prepared, "in-progress").unwrap();
    let mut put = request(None);
    put.provider = "otherdns".into();
    let saving = clients.save(put);
    tokio::pin!(saving);
    if tokio::time::timeout(Duration::from_millis(150), saving.as_mut())
        .await
        .is_ok()
    {
        panic!("签发进行中 save 不该删准备状态、不该完成");
    }
    assert_eq!(fs::read_to_string(&prepared).unwrap(), "in-progress");
    drop(serial);
    assert!(saving.await.unwrap().provider_changed);
    assert!(!prepared.exists());
}

#[tokio::test]
async fn first_provider_requires_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let clients = domain(dir.path());
    let mut put = request(Some("example.test"));
    put.credentials.clear();
    assert!(matches!(
        clients.save(put).await,
        Err(DomainError::Invalid(_))
    ));
    assert!(!dir.path().join("entry/domain.json").exists());
}

#[tokio::test]
async fn changed_provider_requires_its_own_credentials() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let before = fs::read(dir.path().join("entry/domain.json")).unwrap();
    let mut put = request(None);
    put.provider = "cloudflare".into();
    put.credentials.clear();
    assert!(matches!(
        clients.save(put).await,
        Err(DomainError::Invalid(_))
    ));
    assert_eq!(
        fs::read(dir.path().join("entry/domain.json")).unwrap(),
        before
    );
}

#[tokio::test]
async fn unchanged_provider_retains_credentials() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients.save(request(None)).await.unwrap();
    let before = fs::read(dir.path().join("entry/domain.json")).unwrap();
    let mut put = request(None);
    put.credentials.clear();
    assert!(!clients.save(put).await.unwrap().provider_changed);
    assert_eq!(
        fs::read(dir.path().join("entry/domain.json")).unwrap(),
        before
    );
}

/// 排队中的旧签发任务带的是排队时的设置快照：拿到锁后重读现状，变了就跳过——
/// 不然旧快照会重新 prepare，把新服务商的准备状态和证书都写成旧的，续期从此对不上。
#[tokio::test]
async fn queued_issue_skips_when_settings_changed_before_the_lock() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    // 当前设置是 A（httpreq + A 凭据）。
    clients
        .apply(|store| {
            *store = super::Store {
                suffix: Some("abcdefgh.example.test".into()),
                provider: Some("httpreq".into()),
                credentials: [("HTTPREQ_TOKEN".into(), "a-token".into())].into(),
                env_file: None,
                terms_accepted: true,
                last_error: None,
            };
            Ok(())
        })
        .await
        .unwrap();
    // 模拟“排队时抓了 A 快照、还没拿到签发锁”：另一轮 save 已把设置写成 B。
    let stale = clients.effective().unwrap();
    clients
        .apply(|store| {
            store.provider = Some("otherdns".into());
            store.credentials = [("OTHERDNS_TOKEN".into(), "b-token".into())].into();
            Ok(())
        })
        .await
        .unwrap();
    let quiet = clients.engine.recorded_creations().len();
    // 旧任务此刻拿到锁：设置已变，必须跳过，不能按 A 起 lego。
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients.clone().run_issue(stale).await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet,
        "过期快照的任务不该真的起 lego 容器"
    );
    // 对照：按现状排的任务照常签（测试引擎在起容器那步失败，但“试图起”发生）。
    let fresh = clients.effective().unwrap();
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients.clone().run_issue(fresh).await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet + 1,
        "按最新设置排的任务应该照常走签发"
    );
}

/// 上一次签发的结果（last_error）写盘不该把排队的下一次签发误判成“设置变了”：
/// last_error 是签发的输出，不参与签发行为，比较时排除——否则修正凭据后要白等
/// 24 小时的续期循环，设置页还显示着旧失败的报错。
#[tokio::test]
async fn queued_issue_runs_even_after_last_error_changed() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients
        .apply(|store| {
            *store = super::Store {
                suffix: Some("abcdefgh.example.test".into()),
                provider: Some("httpreq".into()),
                credentials: [("HTTPREQ_TOKEN".into(), "a-token".into())].into(),
                env_file: None,
                terms_accepted: true,
                last_error: None,
            };
            Ok(())
        })
        .await
        .unwrap();
    // 上一次签发失败写下的 last_error，快照带着它排队。
    clients
        .update(|state| state.last_error = Some("上一次的失败".into()))
        .await
        .unwrap();
    let queued = clients.effective().unwrap();
    // 上一任务收尾改写了 last_error（成功清空或换成新失败都一样）：设置一个字没变。
    clients
        .update(|state| state.last_error = None)
        .await
        .unwrap();
    let quiet = clients.engine.recorded_creations().len();
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients.clone().run_issue(queued).await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet + 1,
        "last_error 变化不该误杀排队的签发"
    );
}
