//! App 里配的域名（没有设置文件）决定入口：没证书先不建、读不了不删、与设置文件冲突时按设置。

use super::test_support::*;
use crate::entry_compose::HAPROXY_IMAGE;
use crate::entry_runtime::FRPC_IMAGE;

/// “不要名字”的安装没有设置文件：主人在 App 里配了自己的域名（`entry/domain.json`）照样要有入口。
/// 这是 #451 的回归：只看设置文件时这里会按“两个都不要”删。
#[tokio::test]
async fn app_domain_without_settings_creates_entry_and_connector() {
    let rig = Rig::new().await;
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
    // 没有家名服务：连接器是路由器转发用的 HAProxy，不是 frpc。
    assert_eq!(rig.created("wonton-connector")["Image"], HAPROXY_IMAGE);
    // 入口配置按这个域名重写。
    assert!(
        config(&rig).contains("home.example.test"),
        "{}",
        config(&rig)
    );
}

/// 重复对账不重建：每轮都删了再建会让公网入口一直断。
#[tokio::test]
async fn app_domain_without_settings_is_idempotent() {
    let rig = Rig::new().await;
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
    assert_eq!(rig.docker.creations("wonton-connector"), 1);
}

/// 装机时的设置文件没了、App 里配的域名还在：入口容器留着，不按“没有设置”删掉。
#[tokio::test]
async fn app_domain_keeps_containers_after_settings_disappear() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    app_domain(&rig, &settings().domain);
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
    assert_eq!(rig.docker.creations("wonton-connector"), 1);
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
}

/// 域名配好了但证书还没签出来：先不建入口（入口要读 `site.pem` 定运行用户），签出来后的下一轮再建。
#[tokio::test]
async fn app_domain_without_a_certificate_waits_for_the_next_reconcile() {
    let rig = Rig::new().await;
    app_domain(&rig, "home.example.test");
    std::fs::remove_file(rig.dir.path().join("entry/certs/site.pem")).unwrap();
    rig.reconcile(None).await.unwrap();
    assert_eq!(rig.others(), Vec::<String>::new());
    std::fs::write(rig.dir.path().join("entry/certs/site.pem"), "pem").unwrap();
    rig.reconcile(None).await.unwrap();
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
}

/// 域名设置读不了是“不确定”：不按没域名删掉在跑的入口，如实报错让循环退避重试。
#[tokio::test]
async fn an_unreadable_app_domain_keeps_the_containers() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    std::fs::write(rig.dir.path().join("entry/domain.json"), "{").unwrap();
    let error = rig.reconcile(None).await.unwrap_err();
    assert_eq!(error.category(), "invalid_input");
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
}

/// 有设置文件时它说了算：名字和家名服务都按设置，域名设置文件里另写的后缀不参与。
#[tokio::test]
async fn settings_win_over_the_app_domain() {
    let rig = Rig::new().await;
    let mut settings = settings();
    settings.registration = Some(federation_registration());
    let ca = rig.dir.path().join("relay-ca.pem");
    std::fs::write(&ca, "ca").unwrap();
    settings.relay_ca = Some(ca);
    app_domain(&rig, "other.example.test");
    rig.reconcile(Some(&settings)).await.unwrap();
    assert_eq!(rig.created("wonton-connector")["Image"], FRPC_IMAGE);
    assert!(config(&rig).contains(&settings.domain));
    assert!(!config(&rig).contains("other.example.test"));
}
