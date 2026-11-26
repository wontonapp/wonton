//! “HTTPS 和域名”设置页（ADR-0089）的行为测试：表单校验、凭据只上行、固定后缀、证书失败和“其他”服务商。
//! 对面是本机假扮的核心客户端接口。

use axum::http::Method;
use axum::http::StatusCode;
use serde_json::{Value, json};
use wonton_protocol::{
    CaaState, CertificateState, ClientsAction, ClientsDomainState, ClientsDomainViewModel,
    CredentialEntry, DnsProviderPreset, DomainSettings, LanRecordState, SecretString,
    clients_domain_path, dns_providers_path,
};

use crate::plugins::rig::{Canned, Rig, WAIT};

fn presets() -> String {
    serde_json::to_string(&[
        DnsProviderPreset {
            code: "desec".into(),
            name: "deSEC".into(),
            credential_vars: vec!["DESEC_TOKEN".into()],
            signup_url: Some("https://desec.io".into()),
            recommended: true,
        },
        DnsProviderPreset {
            code: "cloudflare".into(),
            name: "Cloudflare".into(),
            credential_vars: vec!["CF_API_TOKEN".into()],
            signup_url: None,
            recommended: false,
        },
    ])
    .unwrap()
}

fn domain(
    provider: Option<&str>,
    suffix_editable: bool,
    credentials_set: bool,
    terms_accepted: bool,
    certificate: CertificateState,
) -> String {
    serde_json::to_string(&DomainSettings {
        suffix: Some("example.test".into()),
        suffix_editable,
        home: None,
        tail: None,
        provider: provider.map(str::to_owned),
        credentials_set,
        terms_accepted,
        certificate,
        caa: CaaState::Missing,
        caa_record: None,
        lan_record: LanRecordState::Ok,
        lan_target: None,
    })
    .unwrap()
}

fn save(
    suffix: Option<&str>,
    provider: Option<&str>,
    credentials: Vec<(&str, &str)>,
    other_credentials: &str,
    accept_terms: bool,
) -> ClientsAction {
    ClientsAction::SaveDomain {
        suffix: suffix.map(str::to_owned),
        provider: provider.map(str::to_owned),
        credentials: credentials
            .into_iter()
            .map(|(name, value)| CredentialEntry {
                name: name.into(),
                value: SecretString::new(value),
            })
            .collect(),
        other_credentials: other_credentials.into(),
        accept_terms,
    }
}

async fn wait_domain(
    rig: &Rig,
    mut done: impl FnMut(&ClientsDomainViewModel) -> bool,
) -> ClientsDomainViewModel {
    let subscription = rig.app.subscribe_clients_domain();
    let mut latest = rig.app.clients_domain();
    let wait = async {
        while let Some(view) = subscription.next().await {
            latest = view;
            if done(&latest) {
                return true;
            }
        }
        false
    };
    match tokio::time::timeout(WAIT, wait).await {
        Ok(true) => latest,
        Ok(false) => panic!("subscription ended"),
        Err(_) => panic!("timed out; latest: {latest:?}"),
    }
}

/// 取好一页表单：服务商预设、已存的域名。
async fn ready(rig: &Rig, domain_json: String) {
    rig.core
        .answer_always(Method::GET, &dns_providers_path(), Canned(200, presets()));
    rig.core.answer_always(
        Method::GET,
        &clients_domain_path(),
        Canned(200, domain_json),
    );
    rig.connected().await;
    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: ClientsAction::RefreshDomain,
        })
        .unwrap();
    wait_domain(rig, |view| view.state == ClientsDomainState::Ready).await;
}

#[tokio::test]
async fn domain_form_requires_terms_before_submit() {
    let rig = Rig::new().await;
    ready(
        &rig,
        domain(Some("desec"), true, false, false, CertificateState::None),
    )
    .await;

    let result = rig.app.dispatch(wonton_client::Action::Clients {
        action: save(
            Some("example.test"),
            Some("desec"),
            vec![("DESEC_TOKEN", "secret")],
            "",
            false,
        ),
    });
    assert!(matches!(
        result,
        Err(wonton_client::CoreError::InvalidInput { .. })
    ));
    assert!(
        rig.core
            .seen(&Method::PUT, &clients_domain_path())
            .is_empty()
    );
}

#[tokio::test]
async fn credentials_never_rendered_back() {
    let rig = Rig::new().await;
    ready(
        &rig,
        domain(Some("desec"), true, true, true, CertificateState::None),
    )
    .await;

    let view = rig.app.clients_domain();
    assert!(view.credentials_set);
    assert_eq!(view.credential_vars, ["DESEC_TOKEN"]);
    let json = serde_json::to_string(&view).unwrap();
    assert!(json.contains("DESEC_TOKEN"));
    assert!(!json.contains("secret"));
}

#[tokio::test]
async fn suffix_hidden_when_fixed() {
    let rig = Rig::new().await;
    ready(
        &rig,
        domain(Some("desec"), false, false, true, CertificateState::None),
    )
    .await;
    assert!(!rig.app.clients_domain().suffix_editable);
    rig.core.answer_always(
        Method::PUT,
        &clients_domain_path(),
        Canned(
            StatusCode::OK.as_u16(),
            domain(Some("desec"), false, false, true, CertificateState::None),
        ),
    );

    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: save(
                Some("other.test"),
                Some("desec"),
                vec![("DESEC_TOKEN", "secret")],
                "",
                true,
            ),
        })
        .unwrap();
    wait_domain(&rig, |view| !view.saving).await;
    let seen = rig.core.seen(&Method::PUT, &clients_domain_path());
    assert_eq!(seen.len(), 1);
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["suffix"], Value::Null);
}

#[tokio::test]
async fn cert_failure_maps_to_retry_with_details() {
    let rig = Rig::new().await;
    ready(
        &rig,
        domain(
            Some("desec"),
            true,
            true,
            true,
            CertificateState::Renewing {
                not_after_ms: 1_700_000_000_000,
                error: "lego boom".into(),
            },
        ),
    )
    .await;

    let view = rig.app.clients_domain();
    assert!(view.certificate.retryable);
    assert_eq!(view.certificate.details.as_deref(), Some("lego boom"));
    rig.core.answer_always(
        Method::PUT,
        &clients_domain_path(),
        Canned(
            StatusCode::OK.as_u16(),
            domain(Some("desec"), true, true, true, CertificateState::Issuing),
        ),
    );

    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: ClientsAction::RetryCertificate,
        })
        .unwrap();
    wait_domain(&rig, |view| !view.saving).await;
    let seen = rig.core.seen(&Method::PUT, &clients_domain_path());
    assert_eq!(seen.len(), 1);
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["provider"], "desec");
    assert_eq!(body["credentials"], json!([]));
    assert_eq!(body["accept_terms"], true);
}

#[tokio::test]
async fn other_provider_parses_key_values() {
    let rig = Rig::new().await;
    ready(
        &rig,
        domain(Some("httpreq"), true, false, true, CertificateState::None),
    )
    .await;
    // 存的是预设以外的服务商：选中的是“其他”。
    assert!(rig.app.clients_domain().other_provider);
    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: ClientsAction::SelectDomainProvider { code: None },
        })
        .unwrap();
    rig.core.answer_always(
        Method::PUT,
        &clients_domain_path(),
        Canned(
            StatusCode::OK.as_u16(),
            domain(Some("httpreq"), true, true, true, CertificateState::None),
        ),
    );

    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: save(
                Some("example.test"),
                Some("httpreq"),
                Vec::new(),
                "HTTPREQ_ENDPOINT=http://127.0.0.1:8055\nFOO=bar",
                true,
            ),
        })
        .unwrap();
    wait_domain(&rig, |view| !view.saving).await;
    let seen = rig.core.seen(&Method::PUT, &clients_domain_path());
    assert_eq!(seen.len(), 1);
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(
        body["credentials"],
        json!([
            {"name": "HTTPREQ_ENDPOINT", "value": "http://127.0.0.1:8055"},
            {"name": "FOO", "value": "bar"},
        ])
    );
}

/// GET 失败要落成失败态（壳才有话和重试），重试成功后回 Ready。
#[tokio::test]
async fn load_failure_is_a_failed_page_and_retry_recovers() {
    let rig = Rig::new().await;
    rig.core
        .answer_always(Method::GET, &dns_providers_path(), Canned(200, presets()));
    rig.core.answer_always(
        Method::GET,
        &clients_domain_path(),
        Canned(
            StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            json!({"error": {"kind": "core_not_in_container", "message": "core is not in its container"}})
                .to_string(),
        ),
    );
    rig.connected().await;
    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: ClientsAction::RefreshDomain,
        })
        .unwrap();
    let failed = wait_domain(&rig, |view| {
        matches!(view.state, ClientsDomainState::Failed { .. })
    })
    .await;
    assert!(
        matches!(&failed.state, ClientsDomainState::Failed { .. }),
        "{failed:?}"
    );

    rig.core.answer_always(
        Method::GET,
        &clients_domain_path(),
        Canned(
            200,
            domain(Some("desec"), true, false, true, CertificateState::None),
        ),
    );
    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: ClientsAction::RefreshDomain,
        })
        .unwrap();
    let ready = wait_domain(&rig, |view| view.state == ClientsDomainState::Ready).await;
    assert_eq!(ready.suffix.as_deref(), Some("example.test"));
}

/// CAA 整条记录和 `*.lan` 的目标地址都由服务端给，客户端不改写。
#[tokio::test]
async fn caa_record_and_lan_target_come_from_the_server() {
    let rig = Rig::new().await;
    let mut json: Value = serde_json::from_str(&domain(
        Some("desec"),
        true,
        true,
        true,
        CertificateState::None,
    ))
    .unwrap();
    json["caa_record"] = Value::String(
        "example.test. CAA 0 issue \"pebble; accounturi=https://pebble:14000/acme/acct/1; validationmethods=dns-01\""
            .into(),
    );
    json["lan_target"] = Value::String("192.0.2.10".into());
    ready(&rig, json.to_string()).await;
    let view = rig.app.clients_domain();
    assert_eq!(
        view.caa_record.as_deref(),
        Some(
            "example.test. CAA 0 issue \"pebble; accounturi=https://pebble:14000/acme/acct/1; validationmethods=dns-01\""
        )
    );
    assert_eq!(view.lan_target.as_deref(), Some("192.0.2.10"));
    assert_eq!(view.lan_record_name.as_deref(), Some("*.lan.example.test"));
}

/// 后缀在客户端就规范成小写、去掉末尾点，再交给核心。
#[tokio::test]
async fn suffix_is_lowercased_and_dot_trimmed() {
    let rig = Rig::new().await;
    ready(
        &rig,
        domain(Some("desec"), true, true, true, CertificateState::None),
    )
    .await;
    rig.core.answer_always(
        Method::PUT,
        &clients_domain_path(),
        Canned(
            200,
            domain(Some("desec"), true, true, true, CertificateState::None),
        ),
    );
    rig.app
        .dispatch(wonton_client::Action::Clients {
            action: save(
                Some("Example.TEST."),
                Some("desec"),
                vec![("DESEC_TOKEN", "secret")],
                "",
                true,
            ),
        })
        .unwrap();
    wait_domain(&rig, |view| !view.saving).await;
    let seen = rig.core.seen(&Method::PUT, &clients_domain_path());
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["suffix"], "example.test");
}
