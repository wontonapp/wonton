//! 确认卡片在本会话里从没结束变成“执行完了”时，已装列表多半还是旧的，卡片上的按钮凭它显示
//! （ADR-0016）：客户端要自己补取一次。启动时的历史结果不补取。测试扮演核心（假核心的确认列表），
//! 数已装列表的请求。卡片不是 Matrix 消息，这些测试不用 Docker。

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wonton_client::{
    Action, Answer, InstalledPlugin, PluginHealth, PluginKind, PluginState, PluginsViewModel,
};

use super::support::{Home, Phone, WAIT};

const PLUGIN: &str = "app.grafana";

/// 钩子报到之后、放行之前，给 settled 的观察窗口：修复前 settled 在这窗口里出现，修复后一直不出现。
/// 实际的先后由钩子和决定的就地性决定，窗口只是给“出现”留够时间，不靠它排先后。
const HELD: Duration = Duration::from_secs(2);

/// 一张装插件的卡片：等回答时带着插件 ID，结束时按 `state` 给结果。
fn card(id: &str, state: Value) -> Value {
    let expires_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 600_000;
    json!({
        "id": id,
        "state": state,
        "title": "Hermes 想安装 Grafana",
        "lines": [],
        "choices": [
            {"answer": "approve", "label": "安装"},
            {"answer": "reject", "label": "不装"},
        ],
        "expires_at_ms": expires_at_ms,
        "plugin_id": PLUGIN,
        "origin": {"room_id": "!hermes:wonton.localhost"},
    })
}

fn pending(id: &str) -> Value {
    card(id, json!({"type": "pending"}))
}

fn succeeded(message: &str) -> Value {
    card("install", json!({"type": "succeeded", "message": message}))
}

fn plugin() -> InstalledPlugin {
    InstalledPlugin {
        chat: Default::default(),
        id: PLUGIN.to_owned(),
        title: "看板".to_owned(),
        summary: String::new(),
        version: "1.0.0".to_owned(),
        revision: "1.0.0".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: wonton_client::PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status: PluginState::Running {
            health: PluginHealth::Healthy,
        },
        web_url: None,
        link_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
    }
}

async fn wait_for<R>(
    phone: &Phone,
    what: &str,
    check: impl Fn(&PluginsViewModel) -> Option<R>,
) -> R {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = check(&phone.plugins()) {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{what}：{WAIT:?} 内没有等到，当前：{:#?}",
            phone.plugins()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 跟随把卡片列进等回答的，才算看见了没结束。
async fn followed_pending(phone: &Phone, id: &str) {
    phone
        .wait_chat("跟随看见卡片在等回答", |view| {
            view.pending_confirmations
                .iter()
                .any(|card| card.id == id)
                .then_some(())
        })
        .await;
}

/// 跟随把卡片列进不用回答的，补不补取就已经定了。
async fn followed_settled(phone: &Phone, id: &str) {
    phone
        .wait_chat("跟随看见卡片有了结果", |view| {
            view.settled_confirmations
                .iter()
                .any(|card| card == id)
                .then_some(())
        })
        .await;
}

async fn connected(phone: &Phone, home: &Home, installed: Vec<InstalledPlugin>) -> usize {
    home.set_installed_plugins(installed);
    // 只要确认列表和已装列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(home);
    phone.dispatch(Action::RefreshPlugins).unwrap();
    wait_for(phone, "连上后的第一次取列表", |view| {
        view.installed.is_some().then_some(())
    })
    .await;
    home.installed_requests()
}

fn approve(phone: &Phone, id: &str) {
    phone
        .dispatch(Action::AnswerConfirmation {
            id: id.to_owned(),
            answer: Answer::Approve,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
}

/// 卡片在这个会话里看着从等回答变成成功：补取一次已装列表，ViewModel 里就有了。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_succeeded_card_fetches_the_installed_list_once() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let baseline = connected(&phone, &home, vec![]).await;

    // 批准后核心的列表把它挪进“执行完了”：客户端自己补取一次。补取时已装列表已经有它了。
    home.set_confirmations(vec![pending("install")], vec![]);
    followed_pending(&phone, "install").await;
    approve(&phone, "install");
    home.set_installed_plugins(vec![plugin()]);
    home.set_confirmations(vec![], vec![succeeded("装好了 Grafana")]);
    followed_settled(&phone, "install").await;
    wait_for(&phone, "补取后的列表", |view| {
        view.installed
            .as_ref()
            .is_some_and(|installed| installed.plugins.iter().any(|plugin| plugin.id == PLUGIN))
            .then_some(())
    })
    .await;
    assert_eq!(home.installed_requests(), baseline + 1);

    // 核心再发一次同样的列表（长轮询回的是全量）：不补取。
    home.set_confirmations(vec![], vec![succeeded("装好了 Grafana")]);
    wait_for(&phone, "重复的列表", |_| {
        (home.confirmations_requests() >= 3).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(home.installed_requests(), baseline + 1);
}

/// 插件已经在已装列表里：卡片成功不补取；卸载后，新的成功卡片恰好补取一次。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn listed_plugins_do_not_refetch_and_missing_ones_only_fetch_once() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let baseline = connected(&phone, &home, vec![plugin()]).await;

    // 插件已经在列表里：卡片成功不补取。
    home.set_confirmations(vec![pending("install")], vec![]);
    followed_pending(&phone, "install").await;
    home.set_confirmations(vec![], vec![succeeded("装好了 Grafana")]);
    followed_settled(&phone, "install").await;
    assert_eq!(home.installed_requests(), baseline);

    // 卸载后（列表里没有了）：另一张成功卡片恰好补取一次，旧卡片不重试、不循环。
    home.set_installed_plugins(vec![]);
    phone.dispatch(Action::RefreshPlugins).unwrap();
    wait_for(&phone, "卸载后的列表", |view| {
        view.installed
            .as_ref()
            .is_some_and(|installed| installed.plugins.iter().all(|plugin| plugin.id != PLUGIN))
            .then_some(())
    })
    .await;
    let after_uninstall = home.installed_requests();
    home.set_confirmations(vec![pending("second")], vec![]);
    followed_pending(&phone, "second").await;
    home.set_confirmations(
        vec![],
        vec![card(
            "second",
            json!({"type": "succeeded", "message": "又装好了"}),
        )],
    );
    // 假核心回的是空列表，列表里不会有这个插件；等补取的请求落地。
    wait_for(&phone, "卸载后补取的请求", |_| {
        (home.installed_requests() == after_uninstall + 1).then_some(())
    })
    .await;
    // 同一张卡片再见到一次（重复的列表）：不再取。
    home.set_confirmations(
        vec![],
        vec![card(
            "second",
            json!({"type": "succeeded", "message": "又装好了"}),
        )],
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(home.installed_requests(), after_uninstall + 1);
}

/// 启动时列表里就直接是历史结果：没有本会话能看见的“没结束 → 成功”，不补取。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn startup_history_results_do_not_fetch() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let baseline = connected(&phone, &home, vec![]).await;

    // 第一份列表里就直接是成功的结果。
    home.set_confirmations(
        vec![],
        vec![
            succeeded("装好了"),
            card("another", json!({"type": "succeeded", "message": "装好了"})),
        ],
    );
    followed_settled(&phone, "install").await;
    followed_settled(&phone, "another").await;
    // 挂着的取列表回来了、下一轮也取过了，补取的请求一次都没多。
    wait_for(&phone, "跟随扫完了列表", |_| {
        (home.confirmations_requests() >= 3).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(home.installed_requests(), baseline);
}

/// settled_confirmations 里出现这张卡片。不带超时：由调用方决定等多久。
/// 先订阅再查现状，不然订阅前的更新会被当成“已经看过”漏掉。
async fn settled_seen(phone: &Phone, id: &str) {
    let subscription = phone.subscribe_chat();
    if phone
        .chat()
        .settled_confirmations
        .iter()
        .any(|card| card == id)
    {
        return;
    }
    while let Some(view) = subscription.next().await {
        if view.settled_confirmations.iter().any(|card| card == id) {
            return;
        }
    }
    panic!("聊天订阅提前结束");
}

/// #565 的不变量：卡片进了 settled_confirmations 时，“补不补取”已经定了。钩子把决定卡在报到和放行
/// 之间：settled 在放行前出现，就是修复前的行为——决定在另一个任务里，settled 谁先谁后看运气；
/// 修复后就地决定，卡着的时候 settled 发不出来。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_card_only_settles_after_the_refetch_decision_was_made() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let baseline = connected(&phone, &home, vec![]).await;

    // 钩子开着：卡片成功后决定先报到、等放行。
    phone.test_arm_card_success_hook();

    home.set_confirmations(vec![pending("install")], vec![]);
    followed_pending(&phone, "install").await;
    // 核心的列表把它直接挪进“执行完了”（比如主人的另一台设备答的）：跟随看见过渡，先决定补不补取。
    home.set_confirmations(vec![], vec![succeeded("装好了 Grafana")]);

    tokio::select! {
        biased;
        // settled 先到，而决定还卡在钩子上没做完：不变量破了。
        _ = settled_seen(&phone, "install") => {
            panic!("the card no longer needs an answer, but the refetch decision has not been made");
        }
        // 决定到了但卡着。修复前 settled 这时已经（或马上）会出现；修复后放行前它出不来。
        _ = phone.test_wait_card_success_arrived() => {
            if tokio::time::timeout(HELD, settled_seen(&phone, "install"))
                .await
                .is_ok()
            {
                panic!(
                    "the card no longer needs an answer, but the refetch decision has not been made"
                );
            }
            phone.test_release_card_success();
        }
        _ = tokio::time::sleep(WAIT) => {
            panic!(
                "neither the hook arrival (decision reached) nor settled (published by the follow) \
                 came within {WAIT:?}"
            );
        }
    }

    // 放行后 settled 才发出来，放行的那次决定也补取了。
    followed_settled(&phone, "install").await;
    wait_for(&phone, "放行后的补取", |_| {
        (home.installed_requests() == baseline + 1).then_some(())
    })
    .await;
}
