//! 入口链路：连接器（PROXY v2）→ 入口 → 辅助容器，签发这一环由 `CertificateManager` 走真 Pebble。

use super::*;

pub(super) async fn issue(case: &mut Case, provider: Provider) -> CertificateManager {
    let install = case.dir.path().join("install");
    std::fs::create_dir_all(install.join("entry/certs")).unwrap();
    let env_file = case.dir.path().join("provider.env");
    let helper_ip = case.helper_ip.clone();
    let pebble_ip = case.pebble_ip.clone();
    write(
        &env_file,
        &format!(
            "HTTPREQ_ENDPOINT=http://{helper_ip}:8484\nHTTPREQ_MODE=RAW\nHTTPREQ_USERNAME={}\nHTTPREQ_PASSWORD={}\n",
            crate::clients::domain::dotenv_value("user$TOKEN"),
            crate::clients::domain::dotenv_value(r#"pass\path$TOKEN "quoted""#),
        ),
    );
    // 先自检辅助容器的写入器和 DNS 转发，和 lego 要走的路径一样。
    let preflight = reqwest::Client::new()
        .post(format!("http://{helper_ip}:8484/present"))
        .basic_auth("user$TOKEN", Some(r#"pass\path$TOKEN "quoted""#))
        .json(&json!({"domain": ZONE, "keyAuth": "preflight"}))
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 200, "辅助写入器不回 200");
    // 自检的那条值要清掉，否则 lego 的传播预检会看到一条对不上的 TXT。
    reqwest::Client::new()
        .post(format!("http://{helper_ip}:8484/cleanup"))
        .basic_auth("user$TOKEN", Some(r#"pass\path$TOKEN "quoted""#))
        .json(&json!({"domain": ZONE, "keyAuth": "preflight"}))
        .send()
        .await
        .unwrap();
    let acme_dns = &case.acme_dns;
    let manager = CertificateManager::new(
        install.clone(),
        case.engine.clone(),
        CertificateConfig {
            directory_url: format!("https://{pebble_ip}:14000/dir"),
            ca_file: Some(case.dir.path().join("pebble/ca.pem")),
            dns_resolvers: case.dns_zone_servers.clone(),
            dns_propagation_disable_ans: true,
            email: None,
            accept_terms: true,
            network: case.network.clone(),
            host_install_dir: install.clone(),
            include_lan: true,
            lan_acme_dns: acme_dns.as_ref().map(|(_, _, lan)| lan.clone()),
            dns_provider: (provider == Provider::HttpReq).then(|| "httpreq".to_owned()),
            dns_env_file: (provider == Provider::HttpReq).then_some(env_file),
        },
    )
    .unwrap();
    let account = manager
        .prepare(
            ZONE,
            acme_dns.as_ref().map(|(_, apex, _)| apex),
            acme_dns.as_ref().map(|(url, _, _)| url.as_str()),
        )
        .await
        .unwrap();
    assert!(account.starts_with("https://"), "账号 URI 不对：{account}");
    if let Err(error) = manager.issue(ZONE).await {
        let (_, helper_logs) = docker(&["logs", "--tail", "40", &case.helper]);
        let (_, chall_logs) = docker(&["logs", "--tail", "20", &case.chall]);
        let env = std::fs::read_to_string(install.join("entry/acme/dns.env")).unwrap_or_default();
        let acme_logs = case
            .acme_dns_container
            .as_deref()
            .map(|name| docker(&["logs", "--tail", "30", name]).1)
            .unwrap_or_default();
        panic!(
            "签发失败：{error:#}\ndns.env:\n{env}\nacme-dns:\n{acme_logs}\nhelper:\n{helper_logs}\nchalltestsrv:\n{chall_logs}"
        );
    }
    case.install = install;
    manager
}
