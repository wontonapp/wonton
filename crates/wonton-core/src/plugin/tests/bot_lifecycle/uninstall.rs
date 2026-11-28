//! 卸载插件：吊销 Bot、停用账号、删除数据。

use super::{Bed, echo, granted, id};
use crate::plugin::store::Store;

#[tokio::test]
async fn uninstall_revokes_the_bot_and_deleting_data_deactivates_its_account() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&echo(), &granted(), super::super::langs())
        .await
        .unwrap();
    let first = bed.env();
    let source_address = bed.source_address();
    bed.matrix
        .as_bot(&first["USER"], async |session| {
            session.set_display_name("自选名字").await
        })
        .await
        .unwrap();

    let report = m
        .uninstall(&id(), false, &super::super::langs())
        .await
        .unwrap();
    let bot = report.bots.first().unwrap().clone();
    assert_eq!(
        (bot.user_id.as_str(), bot.devices_removed, bot.deactivated),
        ("@echo:test", 1, false)
    );
    assert!(
        !bed.token_valid(&first["TOKEN"]),
        "卸载后 Bot 的令牌立即失效"
    );
    assert!(m.source(source_address).is_none());

    // 不删数据重装：同一个账号、同一个主房间，新的设备。
    m.install(&echo(), &granted(), super::super::langs())
        .await
        .unwrap();
    let second = bed.env();
    assert_eq!(
        (&second["USER"], &second["ROOM"]),
        (&first["USER"], &first["ROOM"])
    );
    assert_ne!(second["TOKEN"], first["TOKEN"]);
    assert!(bed.token_valid(&second["TOKEN"]));
    assert_eq!(
        bed.tuwunel
            .state()
            .user(&second["USER"])
            .display_name
            .as_deref(),
        Some("自选名字")
    );

    // 删除数据：账号停用，不可恢复；再装是一个新账号和新的主房间。
    let report = m
        .uninstall(&id(), true, &super::super::langs())
        .await
        .unwrap();
    assert!(report.bots.first().unwrap().deactivated);
    assert!(bed.tuwunel.state().user("@echo:test").deactivated);
    let store = Store::open(bed.dir.path().join("plugins")).unwrap();
    assert!(store.bots().load(&id()).unwrap().bots.is_empty());
    m.install(&echo(), &granted(), super::super::langs())
        .await
        .unwrap();
    let third = bed.env();
    assert_eq!(third["USER"], "@echo-2:test");
    assert_ne!(third["ROOM"], first["ROOM"]);
}
