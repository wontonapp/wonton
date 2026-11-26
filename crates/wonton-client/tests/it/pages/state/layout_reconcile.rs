use super::*;

/// 配置坏着的时候对账跳过了账号数据写回（sync.rs 的 reconcile）；文件修好后，第一个普通页面操作
/// （走 runtime_change）就要把这份配置补上：dirty 置起来，会话来了先推本机的，别的设备的
/// 默认值盖不掉修好的文件。
#[tokio::test]
async fn page_problem_recovery_marks_the_layout_for_push_back() {
    let mut rig = Rig::new().await;
    let layout = rig.data_dir().join("layout.json");
    std::fs::write(&layout, b"{\"server_id\":null}").unwrap();
    rig.reopen();
    assert_eq!(
        rig.app.pages().problem.unwrap().reason,
        Some(PagesFailure::InvalidConfig)
    );
    assert!(!rig.app.test_pages_layout_dirty());

    let fixed = rig.data_dir().join("layout-repaired.json");
    std::fs::write(
        &fixed,
        b"{\"server_id\":null,\"my_apps\":[],\"auto_add\":false}",
    )
    .unwrap();
    std::fs::rename(fixed, layout).unwrap();

    // 第一个普通页面操作（runtime_change）就要接住恢复：补 dirty，别等 5 秒的 recompute 轮询。
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::DismissUninstalledTabs,
        })
        .unwrap();
    assert!(rig.app.test_pages_layout_dirty());
    let recovered = wait_pages(&rig, |view| view.problem.is_none()).await;
    assert!(!recovered.auto_add);
}

/// 没有恢复时，普通操作和刷新不得置 dirty：会话就绪前置了它，reconcile 会推本机的
/// 默认值，盖掉另一台设备已有的配置。
#[tokio::test]
async fn page_problem_normal_actions_do_not_mark_the_layout() {
    let rig = Rig::new().await;
    assert!(!rig.app.test_pages_layout_dirty());
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::DismissUninstalledTabs,
        })
        .unwrap();
    assert!(!rig.app.test_pages_layout_dirty());
}

/// 恢复顺带刷新了配置时也要补置 dirty：修好的文件里带一个已卸载插件的行，refresh_state
/// 删行后 refreshed 也为真——按旧条件 `recovered && !refreshed` 就漏了，会话没就绪时不记脏，
/// 重连对账拿服务器旧配置盖掉修好的文件。会话未就绪时置脏只走 mark_dirty 这一条路，正好观察。
#[tokio::test]
async fn page_problem_recovery_marks_the_layout_even_when_refreshed() {
    let mut rig = Rig::new().await;
    let layout = rig.data_dir().join("layout.json");
    std::fs::write(
        &layout,
        b"{\"server_id\":null,\"my_apps\":[],\"my_apps_initialized\":false}",
    )
    .unwrap();
    rig.reopen();

    // 装插件 A：首份列表初始化把它写进 my_apps 并落盘，快照这份文件字节。
    let plugin_id = "app.recovered-gone";
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    )]]);
    rig.connected().await;
    wait_pages(&rig, |_| rig.app.plugins().installed.is_some()).await;
    tokio::time::timeout(crate::plugins::rig::WAIT, async {
        loop {
            let Ok(content) = std::fs::read_to_string(&layout) else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            if content.contains(plugin_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("自动加入没有落盘");
    let repaired = std::fs::read(&layout).unwrap();

    // 文件坏：进 App（reopen）时读到，进入读不了的状态——运行中坏文件不被发现，
    // 恢复路径（reload_config_if_recovered）接的就是这种启动坏、运行中修好的文件。
    std::fs::write(&layout, b"{").unwrap();
    rig.reopen();
    wait_pages(&rig, |view| view.problem.is_some()).await;

    // A 卸载：读不了期间 refresh_state 跑在内存默认配置上，删不到行，read_problem 保持。
    rig.core.set_lists(vec![vec![]]);
    rig.connected().await;
    wait_pages(&rig, |_| {
        rig.app
            .plugins()
            .installed
            .is_some_and(|installed| installed.plugins.is_empty())
    })
    .await;
    assert!(rig.app.pages().problem.is_some());

    // 文件修好（还是带 A 行的那份字节）：第一个普通页面操作接住恢复，refresh_state 把
    // A 行删掉（refreshed 为真），这份修好的配置照样要记成待推送。
    std::fs::write(&layout, &repaired).unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::DismissUninstalledTabs,
        })
        .unwrap();
    wait_pages(&rig, |view| view.problem.is_none()).await;
    assert!(rig.app.test_pages_layout_dirty());
}

/// 自动加入落盘后账号数据写失败（Tuwunel 整体断开）：就绪分支推送前置了 dirty（push_layout
/// 失败不清它）。恢复后服务器的旧配置再到手机，follow_server 不收它、推本机合并后的配置，旧配置
/// 盖不掉新入口。要真实 Matrix 会话，所以标了 ignore；本机经 cargo-slot 只编不跑，Docker 场景留给 CI。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_failed_layout_push_keeps_the_local_entry_when_the_server_layout_returns() {
    use crate::chat::support::{Home, Phone};

    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;

    let plugin_id = "app.push-failure";
    let mut plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    );
    plugin.web_url = Some(format!("http://{plugin_id}.wonton.localhost/"));

    let other = home.matrix.login("owner", "Other device").await;
    other
        .update_layout(|raw| {
            raw.insert(
                "my_apps".to_owned(),
                serde_json::json!([{
                    "app": {"type": "plugin", "plugin_id": plugin_id},
                    "title": ""
                }]),
            );
        })
        .await
        .unwrap();
    // 旧入口引用的插件也在已装列表里（另一台设备装的），先喂给它建立 previous_plugins
    // 快照：换成不在列表里的入口，对账后的首次刷新会把它当已卸载删掉，场景就走了样。
    home.set_installed_plugins(vec![plugin.clone()]);

    phone.connect(&home);
    phone.signed_in().await;
    // 首次对账收下服务器旧配置并落盘；等落盘出现旧入口，reconcile（layout_ready）才算完，
    // 后面自动加入的推送才一定走就绪分支。
    let layout_path = dir.path().join("servers/1/layout.json");
    tokio::time::timeout(crate::plugins::rig::WAIT, async {
        loop {
            let Ok(content) = std::fs::read_to_string(&layout_path) else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            if content.contains(plugin_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("对账没有收下服务器的旧配置");

    // 让 Tuwunel 整体断开：update_layout 要先读服务器再写，两步都断，推送必失败。
    // set_layout_down 只拦 app.wonton.chat_layout，拦不到这里的 app.wonton.layout。
    home.set_matrix_down(true);
    phone
        .wait_chat("同步因断线停下", |view| {
            matches!(view.sync, wonton_client::ChatSync::Failed { .. }).then_some(())
        })
        .await;

    // 新装的插件到达（主人刷新插件列表），自动加入 append 并落盘；就绪分支置 dirty 后推送，写失败不清它。
    // 已装列表不轮询，没有刷新就到不了手机。
    let mut new_plugin = web_plugin(
        "app.push-failure-new",
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    );
    new_plugin.web_url = Some("http://app.push-failure-new.wonton.localhost/".to_owned());
    home.set_installed_plugins(vec![plugin, new_plugin]);
    phone
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    tokio::time::timeout(crate::plugins::rig::WAIT, async {
        while !phone.test_pages_layout_dirty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("自动加入后没有置 dirty");

    // Tuwunel 恢复时会话还是原来那个，不会重新对账（restart_sync 只在换会话时跑）。服务器的旧配置
    // 再来一次（另一台设备在旧配置上改了别的项）：dirty=true 时 follow_server 不收它，推本机合并后的配置。
    home.set_matrix_down(false);
    other
        .update_layout(|raw| {
            raw.insert("auto_add".to_owned(), serde_json::json!(true));
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + crate::plugins::rig::WAIT;
    loop {
        let server = other.server_layout().await.unwrap();
        let kept = server.as_ref().is_some_and(|content| {
            let apps = content.0.get("my_apps").and_then(|apps| apps.as_array());
            apps.is_some_and(|apps| {
                apps.iter()
                    .any(|entry| entry["app"]["plugin_id"] == plugin_id)
            }) && apps.is_some_and(|apps| {
                apps.iter()
                    .any(|entry| entry["app"]["plugin_id"] == "app.push-failure-new")
            })
        });
        if kept {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "对账没有把本机新入口推上去：{server:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
