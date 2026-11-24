//! 私人 Bot 的持久限制和卷恢复（#658）：撤权或失败后它的卷还在、容器停着，重试接回同一批卷。
//! 在假 Docker 和假 Tuwunel 上验证，真实容器见 [`super::member_bot_recovery_docker`]。

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::super::store::Store;
use super::super::{PluginId, PluginManager, PluginStatus};
use super::bot_lifecycle::Bed;

/// Bot 的数据：一个命名卷、一个匿名卷，镜像还声明了一个匿名卷。
fn bot_package() -> String {
    "services:
  bot:
    image: echo
    volumes: ['data:/named', '/anon']
    environment:
      BOT: ${WONTON_BOT_NAME}
volumes:
  data:
x-wonton:
  spec: '0.5'
  id: app.echo
  version: 1.0.0
  kind: agent
  title: {zh: 测试, en: Test}
  description: {zh: 测试, en: Test}
  bot: {display_name: {zh: 回声, en: Echo}}
  agent: {protocol: matrix}
  bots: {services: [bot]}
  permissions: [matrix.bot]
"
    .to_owned()
}

fn granted() -> Vec<super::super::Permission> {
    vec!["matrix.bot".parse().unwrap()]
}

fn image() -> String {
    "echo".to_owned()
}

fn id() -> PluginId {
    PluginId::parse("app.echo").unwrap()
}

fn forbidden(reason: &str) -> String {
    reason.to_owned()
}

fn named_volume() -> String {
    super::super::names::Instance::default().volume("app-echo", "data")
}

fn container(bot: &str) -> String {
    super::super::names::Instance::default().bot_container("app-echo", "bot", bot)
}

fn stored(bed: &Bed, bot: &str) -> super::super::bots::BotEntry {
    Store::open(bed.dir.path().join("plugins"))
        .unwrap()
        .bots()
        .load(&id())
        .unwrap()
        .bots[bot]
        .clone()
}

/// 容器创建时每个挂载目标接到哪：目标 → 来源（卷名）。
fn mounts(bed: &Bed, bot: &str) -> BTreeMap<String, String> {
    let body = bed.docker.created(&container(bot)).unwrap_or(Value::Null);
    body["HostConfig"]["Mounts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            Some((
                m["Target"].as_str()?.to_owned(),
                m["Source"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

async fn installed(bed: &Bed) -> PluginManager {
    bed.docker.set_image_volumes(&image(), &["/image"]);
    let m = bed.manager();
    m.install(&bot_package(), &granted(), super::langs())
        .await
        .unwrap();
    m
}

async fn added(m: &PluginManager, bot: &str) -> String {
    m.add_bot(
        &id(),
        bot,
        wonton_protocol::BotKeeper::User {
            username: "alice".to_owned(),
        },
    )
    .await
    .unwrap()
    .user_id
}

#[tokio::test]
async fn restricting_a_bot_keeps_its_volumes_and_a_retry_gets_the_same_ones() {
    let bed = Bed::start().await;
    let m = installed(&bed).await;
    added(&m, "alice").await;
    let before = mounts(&bed, "alice");
    // 命名卷有名字，不记；匿名卷和镜像声明的卷记下来。
    assert_eq!(before["/named"], named_volume());
    assert_ne!(before["/anon"], before["/image"]);
    assert!(before["/anon"].starts_with("wonton-anon-"));

    m.restrict_bot(&id(), "alice", forbidden("撤权"))
        .await
        .unwrap();
    assert_eq!(stored(&bed, "alice").restricted.as_deref(), Some("撤权"));
    assert!(!bed.docker.is_running(&container("alice")));
    // 容器还在，匿名卷都记下了；命名卷不进恢复映射。
    assert_eq!(
        stored(&bed, "alice").volumes[&container("alice")],
        BTreeMap::from([
            ("/anon".to_owned(), before["/anon"].clone()),
            ("/image".to_owned(), before["/image"].clone()),
        ])
    );

    // 核心重启：插件标异常、原因写在列表里，待恢复的 Bot 不被起起来。
    let restarted = bed.manager();
    restarted.restore().await.unwrap();
    let PluginStatus::Degraded { reason } = &restarted.list()[0].status else {
        panic!(
            "有待恢复的 Bot 时应标异常：{:?}",
            restarted.list()[0].status
        );
    };
    assert!(
        reason.contains(&super::message(
            "plugin-install-resident-restricted",
            &[("bot", "alice"), ("reason", "撤权")],
        )),
        "{reason}"
    );
    assert!(!bed.docker.is_running(&container("alice")));
    assert_eq!(
        bed.docker.created(&container("alice")).unwrap()["HostConfig"]["Mounts"][0]["Source"],
        before["/named"]
    );

    // 正常重试：同样是那份卷。
    added(&restarted, "alice").await;
    assert_eq!(stored(&bed, "alice").restricted, None);
    assert!(bed.docker.is_running(&container("alice")));
    assert_eq!(mounts(&bed, "alice"), before);
    assert_eq!(bed.docker.creations(&container("alice")), 2);
}

#[tokio::test]
async fn a_failed_retry_keeps_the_recorded_volumes_for_the_next_one() {
    let bed = Bed::start().await;
    let m = installed(&bed).await;
    added(&m, "alice").await;
    let before = mounts(&bed, "alice");
    m.restrict_bot(&id(), "alice", forbidden("撤权"))
        .await
        .unwrap();

    // 重试时容器起不来：新容器回滚掉，卷留在记录里，原因换成这次的错误。
    bed.docker.fail_starts(Some("起不来"));
    let error = m
        .add_bot(
            &id(),
            "alice",
            wonton_protocol::BotKeeper::User {
                username: "alice".to_owned(),
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("起不来"), "{error}");
    assert_eq!(
        stored(&bed, "alice").restricted.as_deref(),
        Some(error.to_string().as_str())
    );
    assert_eq!(
        stored(&bed, "alice").volumes[&container("alice")]["/anon"],
        before["/anon"]
    );
    assert!(!bed.docker.containers().contains(&container("alice")));
    assert!(
        bed.docker.volumes().contains(&before["/anon"]),
        "匿名卷不能跟着回滚没，现有卷：{:?}",
        bed.docker.volumes()
    );

    // 容器没了也要能重启：令牌从那一步起本来就只能等重试，整体核对不能因此失败。
    let restarted = bed.manager();
    restarted.restore().await.unwrap();
    assert!(!bed.docker.containers().contains(&container("alice")));
    bed.docker.fail_starts(None);
    assert!(stored(&bed, "alice").restricted.is_some(), "重启后限制还在");
    added(&m, "alice").await;
    assert_eq!(mounts(&bed, "alice"), before);
    assert!(bed.docker.is_running(&container("alice")));
}

#[tokio::test]
async fn starting_the_plugin_leaves_a_restricted_bot_stopped() {
    let bed = Bed::start().await;
    let m = installed(&bed).await;
    added(&m, "alice").await;
    m.restrict_bot(&id(), "alice", forbidden("撤权"))
        .await
        .unwrap();
    let default = container("default");
    // 全部停掉：插件变成 Stopped。再启动时待恢复的 Bot 不起来，别的照常。
    bed.docker.set_state(
        &default,
        serde_json::json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    let m = Arc::new(bed.manager());
    m.restore().await.unwrap();
    assert_eq!(m.list()[0].status, PluginStatus::Stopped);
    m.start_detached(id()).await.unwrap();
    assert!(bed.docker.is_running(&default));
    assert!(!bed.docker.is_running(&container("alice")));
    assert_eq!(stored(&bed, "alice").restricted.as_deref(), Some("撤权"));
}
