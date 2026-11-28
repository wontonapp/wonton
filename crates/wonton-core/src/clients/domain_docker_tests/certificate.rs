//! 证书本身：路径、权限、属主和 SAN。

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use super::*;

/// 证书叶子里的 DNS 名字。
fn subject_names(pem: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(Ok(der)) = rustls_pemfile::certs(&mut std::io::Cursor::new(pem)).next() {
        let (_, certificate) = x509_parser::parse_x509_certificate(&der).unwrap();
        for name in certificate
            .subject_alternative_name()
            .unwrap()
            .unwrap()
            .value
            .general_names
            .iter()
        {
            if let x509_parser::extensions::GeneralName::DNSName(name) = name {
                names.push((*name).to_owned());
            }
        }
    }
    names
}

pub(super) fn check(case: &Case, manager: &CertificateManager) {
    let install = &case.install;
    let site = manager.certificate_path();
    assert_eq!(site, install.join("entry/certs/site.pem"));
    let pem = std::fs::read(&site).unwrap();
    assert!(pem.starts_with(b"-----BEGIN CERTIFICATE-----"));
    assert!(
        pem.windows(b"-----BEGIN PRIVATE KEY-----".len())
            .any(|part| part == b"-----BEGIN PRIVATE KEY-----")
    );
    let metadata = std::fs::metadata(&site).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(metadata.uid(), std::fs::metadata(install).unwrap().uid());
    let names_in_cert = subject_names(&pem);
    for expected in [
        ZONE.to_owned(),
        format!("*.{ZONE}"),
        format!("*.lan.{ZONE}"),
    ] {
        assert!(
            names_in_cert.contains(&expected),
            "证书没有 {expected}：{names_in_cert:?}"
        );
    }

    // 私钥没挂进 lego：挂载里只有 acme 目录和只读的 CSR。
    let body = manager.container_body(vec!["run".to_owned()]).unwrap();
    let mounts = body.host_config.unwrap().mounts.unwrap();
    assert!(mounts.iter().all(|mount| {
        let source = mount.source.as_deref().unwrap_or_default();
        !source.ends_with("certs") && !source.ends_with("site.key")
    }));
    assert!(mounts.iter().any(|mount| {
        mount.target.as_deref() == Some("/request/site.csr") && mount.read_only == Some(true)
    }));
}
