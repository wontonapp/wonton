use rcgen::KeyPair;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;

fn manager(dir: &Path) -> CertificateManager {
    CertificateManager::new(
        dir.to_owned(),
        Engine::for_test(),
        CertificateConfig {
            directory_url: "https://pebble.test:14000/dir".to_owned(),
            ca_file: None,
            dns_resolvers: vec!["192.0.2.53:53".to_owned()],
            email: None,
            accept_terms: true,
            network: "entry-test-egress".to_owned(),
            host_install_dir: dir.to_owned(),
            include_lan: true,
            lan_acme_dns: Some(credentials("lan")),
            dns_provider: None,
            dns_env_file: None,
        },
    )
    .unwrap()
}

fn credentials(label: &str) -> AcmeDnsCredentials {
    AcmeDnsCredentials {
        username: format!("{label}-user"),
        password: format!("{label}-password"),
        subdomain: label.to_owned(),
        fulldomain: format!("{label}.auth.homes.test"),
    }
}

#[test]
fn preparing_files_preserves_leaf_identity_and_keeps_private_key_out_of_lego() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    manager.initialize_files("abcdefgh.homes.test").unwrap();
    let key_path = manager.entry().join("certs/site.key");
    let key = fs::read(&key_path).unwrap();
    let csr_path = manager.entry().join("requests/site.csr");
    let csr = fs::read(&csr_path).unwrap();
    manager.initialize_files("abcdefgh.homes.test").unwrap();
    assert_eq!(fs::read(&key_path).unwrap(), key);
    assert_eq!(fs::read(&csr_path).unwrap(), csr);
    assert_eq!(
        fs::metadata(key_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let body = manager.container_body(manager.account_command()).unwrap();
    let host = body.host_config.unwrap();
    let mounts = host.mounts.unwrap();
    assert_eq!(mounts.len(), 2);
    assert_eq!(
        mounts[0].source.as_deref(),
        manager.entry().join("acme").to_str()
    );
    assert_eq!(mounts[0].target.as_deref(), Some("/acme"));
    assert_eq!(mounts[0].read_only, Some(false));
    assert_eq!(mounts[1].source.as_deref(), csr_path.to_str());
    assert_eq!(mounts[1].read_only, Some(true));
    assert!(manager.initialize_files("changed.homes.test").is_err());
    fs::remove_file(manager.entry().join("certs/site.key")).unwrap();
    assert!(manager.initialize_files("abcdefgh.homes.test").is_err());
}

#[test]
fn managed_dns_uses_supplied_apex_and_lan_accounts_without_registration_or_cli_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    manager.initialize_files("abcdefgh.homes.test").unwrap();
    manager
        .prepare_dns(
            "abcdefgh.homes.test",
            Some(&credentials("apex")),
            Some("https://names.homes.test/acme"),
        )
        .unwrap();
    let accounts: serde_json::Value =
        serde_json::from_slice(&fs::read(manager.entry().join("acme/dns.json")).unwrap()).unwrap();
    assert_eq!(accounts["abcdefgh.homes.test"]["username"], "apex-user");
    assert_eq!(accounts["*.abcdefgh.homes.test"]["subdomain"], "apex");
    assert_eq!(accounts["lan.abcdefgh.homes.test"]["username"], "lan-user");
    assert_eq!(accounts["*.lan.abcdefgh.homes.test"]["subdomain"], "lan");
    assert_eq!(
        accounts["abcdefgh.homes.test"]["server_url"],
        "https://names.homes.test/acme"
    );
    let body = manager.container_body(manager.account_command()).unwrap();
    assert!(
        !body
            .cmd
            .unwrap()
            .iter()
            .any(|value| value.contains("password"))
    );
    assert!(
        !body
            .env
            .unwrap()
            .iter()
            .any(|value| value.contains("password"))
    );
    assert_eq!(
        fs::metadata(manager.entry().join("acme/dns.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn custom_provider_secrets_are_copied_to_private_env_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager(dir.path());
    let env_file = dir.path().join("provider.env");
    fs::write(&env_file, "CF_DNS_API_TOKEN='test-token'\n").unwrap();
    manager.config.dns_provider = Some("cloudflare".to_owned());
    manager.config.dns_env_file = Some(env_file);
    manager.initialize_files("own.test").unwrap();
    manager.prepare_dns("own.test", None, None).unwrap();
    assert_eq!(
        fs::read_to_string(manager.entry().join("acme/dns.env")).unwrap(),
        "CF_DNS_API_TOKEN='test-token'\n"
    );
    assert_eq!(
        fs::metadata(manager.entry().join("acme/dns.env"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn certificate_publish_is_private_and_failure_preserves_previous_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    manager.initialize_files("abcdefgh.homes.test").unwrap();
    let key =
        KeyPair::from_pem(&fs::read_to_string(manager.entry().join("certs/site.key")).unwrap())
            .unwrap();
    let certificate = rcgen::CertificateParams::new(vec!["abcdefgh.homes.test".to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let output = manager.entry().join("acme/certificates");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("site.crt"), certificate.pem()).unwrap();
    manager.publish_certificate().unwrap();
    let pem = fs::read(manager.certificate_path()).unwrap();
    assert!(pem.starts_with(b"-----BEGIN CERTIFICATE-----"));
    assert!(
        pem.windows(b"-----BEGIN PRIVATE KEY-----".len())
            .any(|part| part == b"-----BEGIN PRIVATE KEY-----")
    );
    assert_eq!(
        fs::metadata(manager.certificate_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fs::write(output.join("site.crt"), "invalid upstream response").unwrap();
    assert!(manager.publish_certificate().is_err());
    assert_eq!(fs::read(manager.certificate_path()).unwrap(), pem);
}

#[tokio::test]
async fn issuance_rejects_missing_preparation_before_contacting_docker() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    let error = manager.issue("abcdefgh.homes.test").await.unwrap_err();
    assert!(error.to_string().contains("prepared"));
}

#[test]
fn private_writes_reject_temporary_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    fs::write(&outside, "unchanged").unwrap();
    symlink(&outside, dir.path().join("site.tmp")).unwrap();
    assert!(files::write_private(&dir.path().join("site.pem"), b"secret").is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "unchanged");
}

#[test]
fn unapproved_terms_and_implicit_public_acme_endpoints_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    let mut config = manager.config.clone();
    config.accept_terms = false;
    assert!(
        CertificateManager::new(
            dir.path().to_owned(),
            manager.engine.clone(),
            config.clone()
        )
        .is_err()
    );
    config.accept_terms = true;
    config.directory_url.clear();
    assert!(
        CertificateManager::new(dir.path().to_owned(), manager.engine.clone(), config).is_err()
    );
}

#[test]
fn preparation_reads_the_registered_v5_account_uri_and_requires_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    manager.initialize_files("abcdefgh.homes.test").unwrap();
    let account_dir = manager
        .entry()
        .join("acme/accounts/pebble.test_14000/wonton");
    fs::create_dir_all(&account_dir).unwrap();
    let key = KeyPair::generate().unwrap();
    files::write_private(
        &account_dir.join("wonton.key"),
        key.serialize_pem().as_bytes(),
    )
    .unwrap();
    fs::write(account_dir.join("account.json"), serde_json::to_vec(&serde_json::json!({
        "server": "https://pebble.test:14000/dir",
        "registration": {"status": "valid", "accountURL": "https://pebble.test:14000/my-account/123"}
    })).unwrap()).unwrap();
    assert_eq!(
        manager.account_uri().unwrap(),
        "https://pebble.test:14000/my-account/123"
    );
    fs::remove_file(account_dir.join("wonton.key")).unwrap();
    assert!(manager.account_uri().is_err());
}
