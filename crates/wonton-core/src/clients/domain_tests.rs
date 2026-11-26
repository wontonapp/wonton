use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use wonton_protocol::{CaaState, CredentialEntry, PutDomainRequest, SecretString};

use super::{DNS_ENV_FILE, Domain, DomainError, dotenv_names, dotenv_value};
use crate::entry_containers::EntryContainers;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;

fn domain(dir: &Path) -> std::sync::Arc<Domain> {
    Domain::new(dir.to_owned(), dir.join("data"), Engine::for_test(), None)
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
    assert!(
        matches!(error, DomainError::Invalid(_) | DomainError::InvalidKey(_)),
        "{error}"
    );
}

#[tokio::test]
async fn provider_name_charset_only() {
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let mut put = request(None);
    put.provider = "Cloudflare_DNS".into();
    assert!(matches!(
        domain(dir.path()).save(put.clone()).await,
        Err(DomainError::Invalid(_) | DomainError::InvalidKey(_))
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
        Err(DomainError::Conflict(_) | DomainError::ConflictKey(_))
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
        Err(DomainError::Conflict(_) | DomainError::ConflictKey(_))
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
        Err(DomainError::Invalid(_) | DomainError::InvalidKey(_))
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
        Err(DomainError::Invalid(_) | DomainError::InvalidKey(_))
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
        Err(DomainError::Invalid(_) | DomainError::InvalidKey(_))
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

/// 续期循环和保存走同一道门：没勾订户协议之前绝不联系 CA，哪怕域名和凭据都齐了。
/// “不要名字”的安装没有设置文件、条款没有环境初值，这条就是它的唯一保险。
#[tokio::test]
async fn renewal_waits_for_the_terms_before_contacting_the_ca() {
    use std::sync::atomic::Ordering;
    unsafe { std::env::set_var("WONTON_ACME_DIRECTORY", "https://pebble.test:14000/dir") };
    let dir = tempfile::tempdir().unwrap();
    let clients = domain(dir.path());
    let settle = async || {
        for _ in 0..20_000 {
            if clients.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("签发没在限定的轮数里结束");
    };
    // 什么都没有：一轮续期不起任何容器。
    clients.renew_if_configured();
    settle().await;
    assert!(clients.engine.recorded_creations().is_empty());
    // 域名、服务商、凭据都有，只差条款：还是不联系 CA。
    clients
        .apply(|store| {
            store.suffix = Some("home.example.test".into());
            store.provider = Some("httpreq".into());
            store.credentials = [("HTTPREQ_ENDPOINT".into(), "x".into())].into();
            Ok(())
        })
        .await
        .unwrap();
    clients.renew_if_configured();
    settle().await;
    assert!(clients.engine.recorded_creations().is_empty());
    // 勾了条款：下一轮续期排到签发，签发在假 Docker 上起 lego 容器时失败。
    clients
        .apply(|store| {
            store.terms_accepted = true;
            Ok(())
        })
        .await
        .unwrap();
    clients.renew_if_configured();
    settle().await;
    assert!(!clients.engine.recorded_creations().is_empty());
}

/// 域名设置文件坏了：发通知、这一轮不联系 CA，也不当没有域名。
#[tokio::test]
async fn an_unreadable_domain_file_is_reported() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let (fake, config) = crate::matrix::fake::start().await;
    let http = crate::http::HttpClient::new();
    let matrix = std::sync::Arc::new(
        crate::matrix::Matrix::connect(&config, dir.path(), http)
            .await
            .unwrap(),
    );
    let notices = std::sync::Arc::new(crate::matrix::NoticeRoom::new(
        matrix,
        "owner",
        dir.path().join("notice-room"),
        std::sync::Arc::new(crate::plugin::languages::Languages::default),
    ));
    let clients = super::Domain::new(
        dir.path().to_owned(),
        dir.path().join("data"),
        Engine::for_test(),
        Some(notices),
    );
    std::fs::create_dir_all(dir.path().join("entry")).unwrap();
    std::fs::write(dir.path().join("entry/domain.json"), "{").unwrap();
    clients.renew_if_configured();
    for _ in 0..20_000 {
        if !fake.state().sent.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let sent = fake.state().sent.clone();
    assert!(
        sent.iter().any(|(_, message)| message["body"]
            .as_str()
            .is_some_and(|body| body.contains("domain settings"))),
        "{sent:?}"
    );
    assert_eq!(clients.pending.load(Ordering::SeqCst), 0);
    assert!(clients.engine.recorded_creations().is_empty());
}

/// 调试用：把域名设置写成这个后缀；`renewable` 时连签发要用的服务商、凭据和条款一起写。
fn app_domain(dir: &Path, suffix: &str, renewable: bool) {
    let mut value = serde_json::json!({"suffix": suffix});
    if renewable {
        value["provider"] = serde_json::json!("httpreq");
        value["credentials"] =
            serde_json::json!({"HTTPREQ_ENDPOINT": "https://relay.test/present"});
        value["terms_accepted"] = serde_json::json!(true);
    }
    fs::write(dir.join("entry/domain.json"), value.to_string()).unwrap();
}

/// 假 lego 只会“成功”：账号和它要发布的新证书自己放。
fn fake_lego_outputs(dir: &Path) {
    let account = dir.join("entry/acme/accounts/pebble.test_14000/wonton");
    fs::create_dir_all(&account).unwrap();
    fs::write(
        account.join("account.json"),
        serde_json::json!({
            "server": "https://pebble.test:14000/dir",
            "registration": {
                "status": "valid",
                "accountURL": "https://pebble.test:14000/my-account/1"
            }
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        account.join("wonton.key"),
        rcgen::KeyPair::generate().unwrap().serialize_pem(),
    )
    .unwrap();
    let new = rcgen::generate_simple_self_signed(vec![
        "example.test".to_owned(),
        "*.example.test".to_owned(),
    ])
    .unwrap();
    fs::create_dir_all(dir.join("entry/acme/certificates")).unwrap();
    fs::write(dir.join("entry/acme/certificates/site.crt"), new.cert.pem()).unwrap();
}

/// 读到换行为止：假的主进程只等一条 `reload` 命令。
async fn read_command(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let mut byte = [0u8; 1];
    loop {
        if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
            break;
        }
    }
}

/// 假的管理端口：每收到一次 reload 就把当时的 crt-list 记一条，立刻回成功。
async fn recording_master(
    crt_list: std::path::PathBuf,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let records: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = records.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
            let crt_list = crt_list.clone();
            tokio::spawn(async move {
                read_command(&mut socket).await;
                let now = match std::fs::read_to_string(&crt_list) {
                    Ok(content) => content,
                    Err(error) => format!("<{error}>"),
                };
                seen.lock().expect("重载记录锁").push(now);
                let _ = socket.write_all(b"Success=1\n").await;
            });
        }
    });
    (addr, records)
}

/// 入口按旧证书跑着、后缀已经改成父域、假 lego 的产物也摆好了：签发前的现场。
async fn renamed_entry_rig() -> (
    FakeDocker,
    std::sync::Arc<EntryContainers>,
    std::sync::Arc<Domain>,
    tempfile::TempDir,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::sync::Arc;
    let (docker, engine) = FakeDocker::start().await;
    docker.add_stranger_container(
        "wonton-core",
        serde_json::json!({"Labels": {"com.docker.compose.project": "wonton"}}),
    );
    for network in ["clients", "edge", "relay"] {
        docker.add_unlabeled_network(&format!("wonton_{network}"));
    }
    let dir = tempfile::tempdir().unwrap();
    let (reload_addr, reloads) =
        recording_master(dir.path().join("entry/certs/public.crt-list")).await;
    let entry = Arc::new(
        EntryContainers::new(
            engine.clone(),
            dir.path().to_owned(),
            dir.path().join("data"),
            crate::plugin::names::Instance::default(),
            Some("wonton-core".to_owned()),
        )
        .with_binding("127.0.0.1", "18443")
        .with_reload_addr(reload_addr),
    );
    let clients = Domain::new(
        dir.path().to_owned(),
        dir.path().join("data"),
        engine.clone(),
        None,
    );
    let old = rcgen::generate_simple_self_signed(vec!["old.example.test".to_owned()]).unwrap();
    fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
    fs::write(dir.path().join("entry/certs/site.pem"), old.cert.pem()).unwrap();
    fs::write(
        dir.path().join("entry/certs/site.key"),
        old.signing_key.serialize_pem(),
    )
    .unwrap();
    app_domain(dir.path(), "old.example.test", false);
    settings(dir.path(), Some("old.example.test"));
    entry.reconcile(None, true).await.unwrap();
    // 后缀改成父域。
    settings(dir.path(), Some("example.test"));
    app_domain(dir.path(), "example.test", true);
    fake_lego_outputs(dir.path());
    (docker, entry, clients, dir, reloads)
}

/// 签发成功后先按新证书写入口配置、再让入口重读：反了入口会拿旧名字的 crt-list 去读新证书，
/// 旧名字被 `strict-sni` 拒、新名字证书对不上。入口收到重载的那一刻就得看到新名字。
#[tokio::test]
async fn a_published_certificate_writes_the_config_before_reloading() {
    use std::sync::atomic::Ordering;
    let (_docker, entry, clients, dir, reloads) = renamed_entry_rig().await;
    assert!(
        reloads.lock().expect("重载记录锁").is_empty(),
        "准备阶段不该重载：{:?}",
        reloads.lock().expect("重载记录锁")
    );
    let served = fs::read_to_string(dir.path().join("entry/certs/public.crt-list")).unwrap();
    assert!(served.contains("old.example.test"), "{served}");
    // 签发成功只叫醒对账，自己不重载。
    clients.renew_if_configured();
    for _ in 0..20_000 {
        if clients.pending.load(Ordering::SeqCst) == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(clients.pending.load(Ordering::SeqCst), 0);
    assert!(
        reloads.lock().expect("重载记录锁").is_empty(),
        "签发成功不该自己先重载"
    );
    // 循环收到唤醒就做这一轮：先按新证书写配置，再重载。重载那一刻看到的名字必须已经是新的。
    entry.reconcile(None, true).await.unwrap();
    let seen = reloads.lock().expect("重载记录锁").clone();
    let last = seen.last().expect("入口没收到重载");
    let names: Vec<&str> = last.split_whitespace().collect();
    assert!(names.contains(&"example.test"), "记录：{seen:?}");
    assert!(!names.contains(&"old.example.test"), "记录：{seen:?}");
}
