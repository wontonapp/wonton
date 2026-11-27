//! 域名设置的保存、签发与续期：校验、凭据、装机初值、排队、发布顺序。

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use wonton_protocol::{CaaState, CredentialEntry, PutDomainRequest, SecretString};

use super::{DNS_ENV_FILE, Domain, DomainError, dotenv_names, dotenv_value};
use crate::entry_containers::EntryContainers;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;

mod caa;
mod credentials;
mod install_env;
mod issue_failure;
mod prepared_issuance;
mod publication;
mod queued_issue;
mod renewal;
mod save_validation;
mod suffix;

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
