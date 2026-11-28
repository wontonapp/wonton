//! Pebble：临时 CA 签发带 IP SAN 的叶证书给自己的 HTTPS，DNS 指到辅助容器。

use super::*;

pub(super) async fn up(case: &mut Case) {
    let pebble_ip = case.pebble_ip.clone();
    let helper_ip = case.helper_ip.clone();
    let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let mut leaf_params = rcgen::CertificateParams::new(vec![pebble_ip.clone()]).unwrap();
    leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    let pebble_tls = case.dir.path().join("pebble");
    write(&pebble_tls.join("cert.pem"), &leaf.pem());
    write(&pebble_tls.join("key.pem"), &leaf_key.serialize_pem());
    write(&pebble_tls.join("ca.pem"), &ca.pem());
    write(
        &pebble_tls.join("pebble.json"),
        &serde_json::to_string(&json!({"pebble": {
            "listenAddress": "0.0.0.0:14000",
            "managementListenAddress": "0.0.0.0:15000",
            "certificate": "/config/cert.pem",
            "privateKey": "/config/key.pem",
            "httpPort": 5002,
            "tlsPort": 5001,
            "externalAccountBindingRequired": false
        }}))
        .unwrap(),
    );

    let pebble_container = case.names.oneshot_container("pebble");
    start(
        &case.names,
        &case.network,
        &pebble_container,
        None,
        &pebble_ip,
        &[],
        &[(pebble_tls, "/config".into())],
        &case.pebble,
        "/app",
        &[
            "-config",
            "/config/pebble.json",
            "-dnsserver",
            &format!("{helper_ip}:53"),
            "-strict=false",
        ],
    );
    wait_http(
        &format!("https://{pebble_ip}:14000/dir"),
        Some(&case.dir.path().join("pebble/ca.pem")),
    )
    .await
    .unwrap();
}
