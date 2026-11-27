//! 证书和 crt-list：换域名、换父域、同名续期都等新证书发布，才换名字并 reload。

use super::test_support::*;

/// 换了后缀但新证书还没发布：入口继续按旧证书里的名字服务，新证书发布后的下一轮才换、才 reload。
#[tokio::test]
async fn a_changed_domain_keeps_the_published_certificate_untill_a_new_one_exists() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["old.example.test", "probe.old.example.test"]);
    app_domain(&rig, "old.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(config(&rig).contains("old.example.test"));
    let reloads = rig.reloads();
    app_domain(&rig, "new.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(
        config(&rig).contains("old.example.test"),
        "{}",
        config(&rig)
    );
    assert!(
        !config(&rig).contains("new.example.test"),
        "{}",
        config(&rig)
    );
    assert_eq!(rig.reloads(), reloads, "名字没换不该 reload");
    publish_cert(&rig, &["new.example.test", "probe.new.example.test"]);
    rig.reconcile(None).await.unwrap();
    assert!(config(&rig).contains("new.example.test"));
    assert!(!config(&rig).contains("old.example.test"));
    assert_eq!(rig.reloads(), reloads + 1);
}

/// 旧证书只有子域、后缀改成它的父域：父域不在旧证书的覆盖范围里，配置留在旧名字；
/// 新证书覆盖父域发布后的下一轮才换。
#[tokio::test]
async fn a_renamed_parent_domain_keeps_the_subdomain_certificate() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["old.example.test", "probe.old.example.test"]);
    app_domain(&rig, "old.example.test");
    rig.reconcile(None).await.unwrap();
    assert!(served_names(&rig).contains(&"old.example.test".to_owned()));
    let reloads = rig.reloads();
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    assert!(
        served_names(&rig).contains(&"old.example.test".to_owned()),
        "{:?}",
        served_names(&rig)
    );
    assert!(
        !served_names(&rig).contains(&"example.test".to_owned()),
        "{:?}",
        served_names(&rig)
    );
    assert_eq!(rig.reloads(), reloads, "名字没换不该 reload");
    publish_cert(&rig, &["example.test", "*.example.test"]);
    rig.reconcile(None).await.unwrap();
    assert!(served_names(&rig).contains(&"example.test".to_owned()));
    assert!(!served_names(&rig).contains(&"old.example.test".to_owned()));
    assert_eq!(rig.reloads(), reloads + 1);
}

/// 同名续期：配置文本一个字都不变，换的只是 `site.pem` 的内容，也必须让入口重读。
#[tokio::test]
async fn a_same_name_renewal_still_reloads() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    let reloads = rig.reloads();
    // 同名、内容不同的新证书：crt-list 的名字和内容都不变。
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        rig.reloads(),
        reloads + 1,
        "证书内容换了，配置文本没变也必须让入口重读"
    );
}
