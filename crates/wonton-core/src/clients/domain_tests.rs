use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use wonton_protocol::{CaaState, CredentialEntry, PutDomainRequest, SecretString};

use super::domain::{Domain, DomainError};
use super::domain::{dotenv_value, read_dotenv};
use crate::plugin::docker::Engine;

fn domain(dir: &Path) -> std::sync::Arc<Domain> {
    Domain::new(
        dir.to_owned(),
        dir.join("data"),
        Engine::for_test(),
        "wonton-test-core".into(),
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
    assert_eq!(caa_state(&[bound], account), CaaState::Bound);
    assert_eq!(caa_state(&[], account), CaaState::Missing);
    let mismatch = CaaIssue {
        tag: "issue".into(),
        issuer: Some("letsencrypt.org".into()),
        parameters: vec![("accounturi".into(), "https://other.test/1".into())],
    };
    assert_eq!(caa_state(&[mismatch], account), CaaState::Mismatch);
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
fn dotenv_values_keep_backslashes_and_dollars() {
    // godotenv 的单引号是字面量：反斜杠和 `$` 原样写、原样读。
    assert_eq!(dotenv_value("plain-token_1"), "plain-token_1");
    assert_eq!(dotenv_value("a\\b$c"), "a\\b$c");
    assert_eq!(dotenv_value("with space"), "'with space'");
    assert_eq!(dotenv_value("a\\b c$d"), "'a\\b c$d'");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dns.env");
    fs::write(&path, "A='a\\b c$d'\nB=a\\b$c\n# comment\n").unwrap();
    let values = read_dotenv(&path).unwrap();
    assert_eq!(values["A"], "a\\b c$d");
    assert_eq!(values["B"], "a\\b$c");
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
