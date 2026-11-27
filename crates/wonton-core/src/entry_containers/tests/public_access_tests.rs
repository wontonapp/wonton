//! 公网访问总开关：关掉撤连接器、清空公网证书列表、重载入口，打开恢复；没有设置文件的装法也要动。

use std::sync::Arc;

use wonton_protocol::GateState;

use crate::plugin::languages::Languages;

use super::test_support::*;

/// 关掉总开关：连接器删掉、入口留着、公网证书列表清空，入口被告知重读配置。
#[tokio::test]
async fn switch_off_removes_the_connector_and_empties_the_public_list() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let reloads = rig.reloads();
    let states = rig
        .containers
        .apply_public_access(Some(&settings()), false, &Languages::default())
        .await;
    assert_eq!(states.connector, GateState::Done);
    assert!(states.crt_list == GateState::Done, "{states:?}");
    assert!(states.core == GateState::Done);
    assert!(
        !rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
    assert_eq!(
        std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list"))
            .unwrap_or_default(),
        ""
    );
    assert_eq!(rig.reloads(), reloads + 1, "入口要重读配置");
}

/// 重新打开：连接器回来、公网证书列表恢复。
#[tokio::test]
async fn switch_on_restores_the_connector_and_the_public_list() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers
        .apply_public_access(Some(&settings()), false, &Languages::default())
        .await;
    let states = rig
        .containers
        .apply_public_access(Some(&settings()), true, &Languages::default())
        .await;
    assert_eq!(states.connector, GateState::Done);
    assert!(states.crt_list == GateState::Done, "{states:?}");
    assert!(
        rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
    let list = std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list")).unwrap();
    assert!(list.contains("k3f9qzab.example.test"), "{list}");
}

/// 同一个开关值重复下发不重建、不重写配置、不重载入口。
#[tokio::test]
async fn the_same_switch_value_does_not_rebuild_or_reload() {
    let rig = Rig::new().await;
    let dir = rig.dir.path().to_owned();
    std::fs::write(
        dir.join("entry/settings.json"),
        serde_json::to_vec(&settings()).unwrap(),
    )
    .unwrap();
    let access = Arc::new(
        crate::entry_access::EntryAccess::open(dir.join("data/core/entry-access.json")).unwrap(),
    );
    access
        .save(wonton_protocol::PublicAccess { enabled: true })
        .unwrap();
    let containers = Arc::new(rig.containers.clone());
    let admin = crate::entry_access::EntryAdmin::new(access, Some(containers), dir);
    admin.apply(true, &Languages::default()).await.unwrap();
    admin.apply(false, &Languages::default()).await.unwrap();
    let creations = rig.docker.creations("wonton-entry");
    let writes = rig.docker.writes().len();
    let reloads = rig.reloads();
    admin.apply(false, &Languages::default()).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-entry"), creations, "不重建");
    assert_eq!(rig.docker.writes().len(), writes, "不重写入口配置");
    assert_eq!(rig.reloads(), reloads, "不重载入口");
}

/// 上一次崩在写盘和重载之间：文件还是旧值，下一次对账改回来并让入口重读一次。
#[tokio::test]
async fn reconcile_fixes_a_stale_list_and_reloads_once() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    std::fs::write(
        rig.dir.path().join("entry/certs/public.crt-list"),
        "stale.example.test\n",
    )
    .unwrap();
    let reloads = rig.reloads();
    rig.reconcile_with(Some(&settings()), false).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list")).unwrap(),
        ""
    );
    assert_eq!(rig.reloads(), reloads + 1, "改了就重读一次");
}

/// 配置没变：第二次对账不重写文件、不重载入口。
#[tokio::test]
async fn an_unchanged_reconcile_does_not_reload() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let reloads = rig.reloads();
    rig.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(rig.reloads(), reloads);
}

/// 公网 crt-list 里现在列了什么。
fn published_public_list(rig: &Rig) -> String {
    std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list")).unwrap_or_default()
}

/// “不要名字”的装法没有入口设置文件，只有 App 里配的域名：总开关照样要撤/建连接器、清/写公网证书列表，
/// 不能因为 settings 为 None 就回 Done 什么都不做（复审 1124 第 1 条）。
#[tokio::test]
async fn switch_without_a_settings_file_runs_the_gates() {
    let rig = Rig::new().await;
    app_domain(&rig, "home.example.test");
    publish_cert(&rig, &["home.example.test"]);
    rig.reconcile(None).await.unwrap();
    assert!(
        rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned()),
        "先按域名对账出连接器"
    );

    // 关：连接器撤掉、公网证书列表清空，两道容器闸门如实报 Done。
    let states = rig
        .containers
        .apply_public_access(None, false, &Languages::default())
        .await;
    assert_eq!(states.connector, GateState::Done, "{states:?}");
    assert_eq!(states.crt_list, GateState::Done, "{states:?}");
    assert!(
        !rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
    assert_eq!(published_public_list(&rig), "");

    // 开：连接器回来、证书列表按域名恢复。
    let states = rig
        .containers
        .apply_public_access(None, true, &Languages::default())
        .await;
    assert_eq!(states.connector, GateState::Done, "{states:?}");
    assert_eq!(states.crt_list, GateState::Done, "{states:?}");
    assert!(
        rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
    assert!(
        published_public_list(&rig).contains("home.example.test"),
        "{}",
        published_public_list(&rig)
    );
}
