//! 聊天列表最上面的安装占位行（ADR-0067）：装的时候有阶段，失败给类别和重试的来源，清掉任务就撤行。

use super::*;
use wonton_protocol::{BotInstallRow, BotInstallState, InstallProgress, InstallStage};

/// 等聊天 ViewModel 出现满足条件的行。
async fn row(
    rig: &Rig,
    chats: &wonton_client::ChatSubscription,
    done: impl Fn(&BotInstallRow) -> bool,
) -> BotInstallRow {
    tokio::time::timeout(WAIT, async {
        loop {
            let view = chats.next().await.expect("订阅提前结束");
            if let Some(row) = view.installing_bots.iter().find(|row| done(row)) {
                return row.clone();
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("等不到安装行：{:?}", rig.app.chat().installing_bots))
}

#[tokio::test]
async fn an_installing_bot_shows_its_stage_from_the_plugin_list() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut installing = installed("app.hermes", PluginState::Installing);
    installing.chat = Some(Default::default());
    installing.install_progress = Some(InstallProgress {
        stage: InstallStage::PullingImages,
        percent: Some(40),
    });
    rig.core.set_lists(vec![vec![installing]]);
    let chats = rig.app.subscribe_chat();
    rig.app.dispatch(Action::RefreshPlugins).unwrap();

    let shown = row(&rig, &chats, |row| row.plugin_id == "app.hermes").await;
    assert_eq!(shown.title, "标题 app.hermes");
    assert_eq!(shown.operation, PluginOperation::Install);
    assert!(matches!(
        &shown.state,
        BotInstallState::Running { progress }
            if progress.stage == InstallStage::PullingImages && progress.percent == Some(40)
    ));
}

#[tokio::test]
async fn a_failed_install_leaves_a_retry_row_until_dismissed() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut shown = preview("app.hermes");
    shown.has_bot = true;
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        core_error(500, "upstream", "Docker 报错"),
    );
    let chats = rig.app.subscribe_chat();
    rig.app
        .dispatch(Action::InstallPlugin {
            preview: shown,
            secrets: Default::default(),
        })
        .unwrap();

    let failed = row(&rig, &chats, |row| {
        matches!(row.state, BotInstallState::Failed { .. })
    })
    .await;
    assert_eq!(
        failed.state,
        BotInstallState::Failed {
            reason: PluginFailure::Upstream,
            message: "Docker 报错".to_owned(),
        }
    );
    assert_eq!(
        failed.source.as_deref(),
        Some("github:owner/app.hermes@v1.0.0"),
        "重试要能拿回来源"
    );

    rig.app
        .dispatch(Action::DismissPluginTask {
            id: "app.hermes".to_owned(),
        })
        .unwrap();
    tokio::time::timeout(WAIT, async {
        loop {
            if chats
                .next()
                .await
                .expect("订阅提前结束")
                .installing_bots
                .is_empty()
            {
                return;
            }
        }
    })
    .await
    .expect("清掉任务后不再留行");
}
