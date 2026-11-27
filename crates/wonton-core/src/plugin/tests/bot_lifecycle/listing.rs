//! 列表里的 Bot 标记：共享 Bot 和危险模式。

use wonton_protocol::BotKeeper;

use super::{Bed, agent, granted_with_tools, id};

/// 列表里的默认 Bot 带“是共享 Bot 还是私人 Bot”和“危险模式开着没”（ADR-0088、ADR-0091）。
#[tokio::test]
async fn listings_mark_shared_bots_and_danger_mode() {
    let bed = Bed::start().await;
    let m = bed.manager();
    m.install(&agent(), &granted_with_tools(), super::super::langs())
        .await
        .unwrap();
    let owner_listing = m.installed_plugin(&id(), &super::super::langs()).unwrap();
    assert!(!owner_listing.bot_shared, "默认 Bot 先是主人的私人 Bot");
    assert!(owner_listing.bot_danger_can_toggle);

    // 家人的私人 Bot 不算主人的，开关不显示（ADR-0091）。
    let mut bots = m.store().bots().load(&id()).unwrap();
    bots.bots.get_mut("default").unwrap().keeper = BotKeeper::User {
        username: "family".to_owned(),
    };
    m.store().bots().save(&id(), &bots).unwrap();
    let family_listing = m.installed_plugin(&id(), &super::super::langs()).unwrap();
    assert!(!family_listing.bot_danger_can_toggle);
    bots.bots.get_mut("default").unwrap().keeper = BotKeeper::User {
        username: "owner".to_owned(),
    };
    m.store().bots().save(&id(), &bots).unwrap();

    m.add_bot(&id(), "family", BotKeeper::Household)
        .await
        .unwrap();
    m.tool_policy()
        .set_danger(id().as_str(), "default", true)
        .unwrap();
    let listing = m.installed_plugin(&id(), &super::super::langs()).unwrap();
    assert!(
        !listing.bot_shared,
        "共享 Bot 是额外的 Bot，默认 Bot 仍是主人的"
    );
    assert!(listing.bot_danger, "危险模式应由列表带出来");
}
