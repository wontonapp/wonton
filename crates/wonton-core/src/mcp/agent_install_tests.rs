//! Agent 从 outbox 装和更新自己写的插件（ADR-0033）：判定、暂存、卡片（最高级两步）、归属和日志。
//! 都不写文案断言：断言错误类别、有没有卡片、副作用。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{Answer, ConfirmationState, PluginTrust};

use super::tests::{Setup, failed, setup, text};
use crate::plugin::PluginId;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;

pub(super) const INSTALL: &str = "wonton__install_plugin";

pub(super) struct Bench {
    pub(super) s: Setup,
    pub(super) outbox: PathBuf,
}

pub(super) async fn bench() -> Bench {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let id = PluginId::parse("com.example.agent").unwrap();
    let outbox = s.bus.plugins.app_data_dir(&id).join("outbox");
    fs::create_dir_all(&outbox).unwrap();
    // 主人的语言：生产里由 App 写入，卡片和提示按它出中文。
    s.bus.plugins.note_person_languages("owner", Some("zh"));
    Bench { s, outbox }
}

impl Bench {
    /// 写一个插件目录：`manifest` 是 x-wonton 之外的 compose 片段，`x_wonton` 是清单字段。
    /// 顺带写上 compose 引用的 index.html（清单里的 configs 引用它）。
    pub(super) fn write_package(&self, dir: &str, services: &str, x_wonton: &str) {
        let compose = format!("name: app\n{services}\nx-wonton:\n{x_wonton}");
        self.write(dir, "docker-compose.yml", &compose);
        self.write(dir, "index.html", "<h1>你好</h1>");
    }

    fn write(&self, dir: &str, path: &str, content: &str) {
        let path = self.outbox.join(dir).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn other(&self) -> crate::mcp::Caller {
        let title = [("zh".to_owned(), "别的 Bot".to_owned())].into();
        crate::mcp::Caller::new(PluginId::parse("com.example.other").unwrap(), Some(&title))
    }

    /// 在后台以另一个身份调用，好在等确认时回答。
    fn spawn_as(
        &self,
        caller: &crate::mcp::Caller,
        tool: &str,
        args: Value,
    ) -> tokio::task::JoinHandle<rmcp::model::CallToolResult> {
        let (bus, tool, caller) = (self.s.bus.clone(), tool.to_owned(), caller.clone());
        let request = rmcp::model::CallToolRequestParams::new(tool)
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        tokio::spawn(async move { bus.call(&caller, request, std::future::pending()).await })
    }

    /// 发卡片的安装：回答第 `nth` 张卡片，返回那次调用的结果。
    pub(super) async fn installed_with_answer(&self, dir: &str, nth: usize, answer: Answer) {
        let call = self.s.spawn(INSTALL, json!({"dir": dir}));
        let card = self.s.cards.wait_for(nth).await;
        self.s.answer(&card.id, answer).unwrap();
        call.await.unwrap();
    }

    pub(super) fn writer(&self, id: &str) -> Option<wonton_protocol::PluginWriter> {
        let id = PluginId::parse(id).unwrap();
        self.s.bus.plugins.writer(&id).unwrap()
    }

    pub(super) fn installed(&self, id: &str) -> bool {
        self.s.manager.list().iter().any(|p| p.id.as_str() == id)
    }
}

/// docker.socket 权限要配上对应的挂载（ADR-0036），卡片前的 fits_host 会核对。
pub(super) fn plain_root(id: &str, version: &str) -> (&'static str, String) {
    let (_, x_wonton) = plain(id, version, "docker.socket");
    let services = "services:
  web:
    image: nginx:1.29-alpine
    init: true
    configs:
      - {source: page, target: /srv/index.html}
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
configs:
  page: {file: index.html}
";
    (services, x_wonton)
}

pub(super) fn plain(id: &str, version: &str, permissions: &str) -> (&'static str, String) {
    // 没有 webui 和健康检查：FakeDocker 起不了网页，等健康检查只会超时。
    // index.html 挂进插件包，摘要跟着内容走。
    (
        "services:
  web:
    image: nginx:1.29-alpine
    init: true
    configs:
      - {source: page, target: /srv/index.html}
configs:
  page: {file: index.html}
",
        format!(
            "  spec: '0.5'\n  id: {id}\n  version: {version}\n  kind: app\n  \
             title: {{zh: 你好, en: Hello}}\n  description: {{zh: Agent 写的页面, en: A page}}\n  \
             permissions: [{permissions}]\n"
        ),
    )
}

/// 第一次装：有卡片；批准后 written_by 是这个 Bot，trust 是 Local，origin 为空；拒绝则什么都没装。
#[tokio::test]
async fn a_first_install_needs_the_owner_and_records_the_writer() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);

    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    assert!(
        b.s.manager
            .list()
            .iter()
            .all(|p| p.id.as_str() != "app.agent.hello")
    );
    b.s.answer(&card.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert!(failed(&result), "{result:?}");
    assert!(!b.installed("app.agent.hello"));
    assert_eq!(b.writer("app.agent.hello"), None);
    assert!(
        b.s.cards.posted.lock().unwrap().len() == 1,
        "拒绝后没有新卡片"
    );

    // 再来一次，这次批准。
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(2).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.agent.hello"));
    let writer = b.writer("app.agent.hello").unwrap();
    assert_eq!(writer.plugin_id, "com.example.agent");
    assert_eq!(writer.title, "测试 Bot");
    let langs = Languages::from_header(Some("zh"));
    let plugin =
        b.s.manager
            .installed_plugin(&PluginId::parse("app.agent.hello").unwrap(), &langs)
            .unwrap();
    assert_eq!(plugin.trust, PluginTrust::Local);
    let record =
        b.s.bus
            .plugins
            .installed_record(&PluginId::parse("app.agent.hello").unwrap())
            .unwrap()
            .unwrap();
    assert!(record.origin.is_none());
}

/// 最高级：两张卡片。第一张批准、第二张没批之前没有副作用；第二张拒绝时什么都没装。
#[tokio::test]
async fn a_highest_permission_install_needs_both_cards() {
    let b = bench().await;
    let (services, x_wonton) = (
        "services:
  web:
    image: nginx:1.29-alpine
    init: true
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
",
        "  spec: '0.5'\n  id: app.agent.root\n  version: 0.1.0\n  kind: app\n  \
         title: {zh: 全能, en: Root}\n  description: {zh: 要 Docker, en: Needs Docker}\n  \
         permissions: [docker.socket]\n"
            .to_owned(),
    );
    b.write_package("root", services, &x_wonton);

    let call = b.s.spawn(INSTALL, json!({"dir": "root"}));
    let first = b.s.cards.wait_for(1).await;
    b.s.answer(&first.id, Answer::Approve).unwrap();
    // 第二张没批之前：什么都没装。
    let second = b.s.cards.wait_for(2).await;
    assert!(!b.installed("app.agent.root"), "第二张批准之前没有副作用");
    b.s.answer(&second.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert!(failed(&result), "{result:?}");
    assert!(!b.installed("app.agent.root"));
    let states = b.s.cards.states(&first.id);
    assert!(
        matches!(states.last(), Some(ConfirmationState::Failed { .. })),
        "{states:?}"
    );

    // 两张都同意才装上。
    let call = b.s.spawn(INSTALL, json!({"dir": "root"}));
    let first = b.s.cards.wait_for(3).await;
    b.s.answer(&first.id, Answer::Approve).unwrap();
    let second = b.s.cards.wait_for(4).await;
    b.s.answer(&second.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.agent.root"));
}

/// 确认期间改目录：装上的还是确认时的那份（暂存的包）。
#[tokio::test]
async fn what_the_owner_approved_is_what_installs() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    b.write("hello", "index.html", "<h1>确认时</h1>");

    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    // 确认期间改了目录：不影响装上的那份。
    b.write("hello", "index.html", "<h1>确认之后改的</h1>");
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.agent.hello"));
    let record =
        b.s.bus
            .plugins
            .installed_record(&PluginId::parse("app.agent.hello").unwrap())
            .unwrap()
            .unwrap();
    let files = record.files.get("index.html").unwrap();
    assert_eq!(files, "<h1>确认时</h1>".as_bytes());
}

/// 新提交替换旧的：旧调用返回“被取代”的错误，旧卡片变成被取代，只有新的能批。
#[tokio::test]
async fn a_new_submission_supersedes_the_old_one() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);

    let first = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let old_card = b.s.cards.wait_for(1).await;
    let second = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let new_card = b.s.cards.wait_for(2).await;

    // 旧的那次调用拿到能分清的错误，旧卡片被取代。
    let result = first.await.unwrap();
    assert!(failed(&result), "{result:?}");
    assert!(text(&result).contains("取代"), "{result:?}");
    let states = b.s.cards.states(&old_card.id);
    assert!(
        matches!(states.last(), Some(ConfirmationState::Failed { .. })),
        "{states:?}"
    );

    // 只有新的能批。
    b.s.answer(&old_card.id, Answer::Approve).unwrap_err();
    b.s.answer(&new_card.id, Answer::Approve).unwrap();
    let result = second.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.agent.hello"));
}

/// 占用：官方装的、别的 Bot 写的，别的 Bot 来更新都报 invalid_input 且零卡片。
#[tokio::test]
async fn occupied_ids_are_refused_without_a_card() {
    let b = bench().await;
    // 官方索引装的：Agent 写一个同 id 的包，是占用，不是更新。
    let call = b.s.spawn(INSTALL, json!({"id": "app.example.journal"}));
    let card = b.s.cards.wait_for(1).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    call.await.unwrap();

    let (services, x_wonton) = plain("app.example.journal", "9.9.9", "");
    b.write_package("occupied", services, &x_wonton);
    let before = b.s.cards.posted.lock().unwrap().len();
    let result = b.s.call(INSTALL, json!({"dir": "occupied"})).await;
    assert!(
        failed(&result) && text(&result).contains("invalid_input"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before, "零卡片");

    // 别的 Bot 写的：让另一个 Bot 装（它有自己的 outbox），这个 Bot 来更新。
    let other = b.other();
    let other_outbox = b.s.bus.plugins.app_data_dir(&other.id).join("outbox");
    fs::create_dir_all(other_outbox.join("theirs")).unwrap();
    let (services, x_wonton) = plain("app.agent.theirs", "0.1.0", "");
    fs::write(
        other_outbox.join("theirs/docker-compose.yml"),
        format!("name: app\n{services}\nx-wonton:\n{x_wonton}"),
    )
    .unwrap();
    fs::write(other_outbox.join("theirs/index.html"), "<h1>他们的</h1>").unwrap();
    let other_call = b.spawn_as(&other, INSTALL, json!({"dir": "theirs"}));
    let card = b.s.cards.wait_for(before + 1).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = other_call.await.unwrap();
    assert!(!failed(&result), "{result:?}");

    let result = b.s.call(INSTALL, json!({"dir": "theirs"})).await;
    assert!(
        failed(&result) && text(&result).contains("invalid_input"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before + 1, "零卡片");
}

/// 更新：权限没变宽时零卡片、有审计、记录已更新，归属还在；变宽时卡片只列新增的项。
#[tokio::test]
async fn an_update_without_new_permissions_skips_the_card() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    b.write("hello", "index.html", "<h1>一</h1>");
    b.installed_with_answer("hello", 1, Answer::Approve).await;
    let writer = b.writer("app.agent.hello").unwrap();

    // 同版本、内容不同：直接更新，零卡片，有审计。
    b.write("hello", "index.html", "<h1>二</h1>");
    let before = b.s.cards.posted.lock().unwrap().len();
    let result = b.s.call(INSTALL, json!({"dir": "hello"})).await;
    assert!(!failed(&result), "{result:?}");
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before, "零卡片");
    let audit = b.s.audit();
    assert_eq!(audit.last().unwrap()["decision"], "allowed");
    let record =
        b.s.bus
            .plugins
            .installed_record(&PluginId::parse("app.agent.hello").unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(
        record.files.get("index.html").unwrap(),
        "<h1>二</h1>".as_bytes()
    );
    assert_eq!(
        b.writer("app.agent.hello").unwrap().plugin_id,
        writer.plugin_id
    );

    // 摘要相同：报没有变化。
    let result = b.s.call(INSTALL, json!({"dir": "hello"})).await;
    assert!(
        failed(&result) && text(&result).contains("没有变化"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before);

    // 权限变宽：卡片只列新增的项（documents 是新增的；已授的 tools 不再列）。
    let (services, x_wonton) = plain(
        "app.agent.hello",
        "0.2.0",
        "tools, storage.share.documents:read",
    );
    b.write_package("hello", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(2).await;
    let joined = card
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("documents"), "{joined}");
    assert!(joined.contains("工具"), "{joined}");
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.writer("app.agent.hello").is_some(), "更新后归属还在");
}

/// 很高或最高级：没变宽也每次发卡片；最高级的更新两张卡。
#[tokio::test]
async fn risky_updates_always_ask_even_without_widening() {
    let b = bench().await;
    let (services, x_wonton) = (
        "services:
  web:
    image: nginx:1.29-alpine
    init: true
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
",
        "  spec: '0.5'\n  id: app.agent.root\n  version: 0.1.0\n  kind: app\n  \
         title: {zh: 全能, en: Root}\n  description: {zh: 要 Docker, en: Needs Docker}\n  \
         permissions: [docker.socket]\n"
            .to_owned(),
    );
    b.write_package("root", services, &x_wonton);
    // 第一次：两张卡。
    let call = b.s.spawn(INSTALL, json!({"dir": "root"}));
    b.s.answer(&b.s.cards.wait_for(1).await.id, Answer::Approve)
        .unwrap();
    b.s.answer(&b.s.cards.wait_for(2).await.id, Answer::Approve)
        .unwrap();
    call.await.unwrap();

    // 内容变了、权限没变：更新仍然两张卡。
    let (services, x_wonton) = (
        "services:
  web:
    image: nginx:1.30-alpine
    init: true
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
",
        "  spec: '0.5'\n  id: app.agent.root\n  version: 0.1.1\n  kind: app\n  \
         title: {zh: 全能, en: Root}\n  description: {zh: 要 Docker, en: Needs Docker}\n  \
         permissions: [docker.socket]\n"
            .to_owned(),
    );
    b.write_package("root", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "root"}));
    b.s.answer(&b.s.cards.wait_for(3).await.id, Answer::Approve)
        .unwrap();
    b.s.answer(&b.s.cards.wait_for(4).await.id, Answer::Approve)
        .unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(b.installed("app.agent.root"));
}

/// 主人打开“每次都问”后，权限没变宽的更新也发卡片。
#[tokio::test]
async fn ask_every_update_brings_the_card_back() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    b.installed_with_answer("hello", 1, Answer::Approve).await;

    let id = PluginId::parse("app.agent.hello").unwrap();
    let langs = Languages::from_header(Some("zh"));
    b.s.manager.set_ask_every_update(&id, true, &langs).unwrap();

    b.write("hello", "index.html", "<h1>三</h1>");
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(2).await;
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
}

/// 主人经文件或来源升级后，归属清掉：原来的 Bot 再更新报 invalid_input。
#[tokio::test]
async fn an_owner_upgrade_takes_the_plugin_back_from_the_agent() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    b.installed_with_answer("hello", 1, Answer::Approve).await;
    let id = PluginId::parse("app.agent.hello").unwrap();
    assert!(b.writer("app.agent.hello").is_some());

    // App 的升级路径升级成功后清归属（和网关 handler 一样调 set_writer）。
    let (services, x_wonton) = plain("app.agent.hello", "0.2.0", "");
    let compose = format!("name: app\n{services}\nx-wonton:\n{x_wonton}");
    let mut files = std::collections::BTreeMap::new();
    files.insert(
        "index.html".to_owned(),
        "<h1>主人升的</h1>".as_bytes().to_vec(),
    );
    let package = crate::plugin::PluginPackage::with_files(
        &compose,
        files,
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let granted = package.manifest.permissions.clone();
    b.s.manager
        .upgrade_upload(
            &id,
            package,
            granted,
            false,
            &Languages::from_header(Some("zh")),
        )
        .await
        .unwrap();
    b.s.manager.set_writer(&id, None).unwrap();

    let (services, x_wonton) = plain("app.agent.hello", "0.3.0", "");
    b.write_package("hello", services, &x_wonton);
    let before = b.s.cards.posted.lock().unwrap().len();
    let result = b.s.call(INSTALL, json!({"dir": "hello"})).await;
    assert!(
        failed(&result) && text(&result).contains("invalid_input"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before, "零卡片");
}

/// 安装和更新的卡片（含最高级的第二张）只有“装/不装”两个选择，没有放宽；主人开了放行时段、
/// 把工具设成“一律允许”之后，安装、变宽的更新和最高级的更新都还是要主人点。
#[tokio::test]
async fn a_window_and_always_allow_do_not_relax_installs() {
    let b = bench().await;
    let only_yes_no = |card: &wonton_protocol::ConfirmationCard| {
        let answers: Vec<_> = card.choices.iter().map(|c| c.answer).collect();
        assert_eq!(
            answers,
            vec![Answer::Approve, Answer::Reject],
            "{}",
            card.title
        );
    };
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    // 先开一个放行时段：对会改数据的插件工具回答“N 分钟内都允许”，并把工具设成一律允许。
    let call = b.s.spawn("app-example-notes__delete", json!({"name": "x"}));
    let card = b.s.cards.wait_for(1).await;
    assert!(
        card.choices.iter().any(|c| c.answer == Answer::AllowWindow),
        "工具卡能开放行时段",
    );
    b.s.answer(&card.id, Answer::AllowWindow).unwrap();
    call.await.unwrap();
    b.s.bus
        .change_tool_policy(
            super::PolicyChange::Always {
                plugin: "app.example.notes",
                caller: "com.example.agent",
            },
            true,
            "owner",
        )
        .unwrap();

    // 第一次安装照旧要卡片，也只有装/不装。
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(2).await;
    only_yes_no(&card);
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");

    // 权限变宽的更新照旧要卡片。
    let (services, x_wonton) = plain("app.agent.hello", "0.2.0", "storage.share.documents:read");
    b.write_package("hello", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(3).await;
    only_yes_no(&card);
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");

    // 最高级的更新：两张卡，都没有放宽。
    let (services, x_wonton) = plain_root("app.agent.hello", "0.3.0");
    b.write_package("hello", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(4).await;
    only_yes_no(&card);
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let card = b.s.cards.wait_for(5).await;
    only_yes_no(&card);
    b.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
}

/// 路径规则和 check_plugin 同一套：软链接的目录被拒，零卡片。
#[tokio::test]
async fn a_symlinked_directory_is_refused() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("real", services, &x_wonton);
    #[cfg(unix)]
    std::os::unix::fs::symlink(b.outbox.join("real"), b.outbox.join("alias")).unwrap();
    let before = b.s.cards.posted.lock().unwrap().len();
    let result = b.s.call(INSTALL, json!({"dir": "alias"})).await;
    assert!(
        failed(&result) && text(&result).contains("permission_denied"),
        "{result:?}"
    );
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before, "零卡片");
}

/// plugin_status：写它的 Bot 看得到 logs，别的 Bot 看不到。
#[tokio::test]
async fn logs_are_only_for_the_bot_that_wrote_it() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    b.installed_with_answer("hello", 1, Answer::Approve).await;
    let id = PluginId::parse("app.agent.hello").unwrap();
    let _ = id;

    let writer_status =
        b.s.call("wonton__plugin_status", json!({"id": "app.agent.hello"}))
            .await;
    let value: Value = serde_json::from_str(&text(&writer_status)).unwrap();
    assert!(value["logs"].is_array(), "{value}");

    let other = b.other();
    let request = rmcp::model::CallToolRequestParams::new("wonton__plugin_status".to_owned())
        .with_arguments(
            json!({"id": "app.agent.hello"})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
    let other_status = b.s.bus.call(&other, request, std::future::pending()).await;
    let value: Value = serde_json::from_str(&text(&other_status)).unwrap();
    assert!(value["logs"].is_null(), "{value}");
}

/// 卡片上有 plugin_id，壳好画“打开”“放到底栏”。
#[tokio::test]
async fn install_cards_carry_the_plugin_id() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    let call = b.s.spawn(INSTALL, json!({"dir": "hello"}));
    let card = b.s.cards.wait_for(1).await;
    assert_eq!(card.plugin_id.as_deref(), Some("app.agent.hello"));
    b.s.answer(&card.id, Answer::Approve).unwrap();
    call.await.unwrap();

    // id 路径的卡片也填。
    let call = b.s.spawn(INSTALL, json!({"id": "app.example.journal"}));
    let card = b.s.cards.wait_for(2).await;
    assert_eq!(card.plugin_id.as_deref(), Some("app.example.journal"));
    b.s.answer(&card.id, Answer::Approve).unwrap();
    call.await.unwrap();
}

/// id 和 dir 同时给、都不给：invalid_input，零卡片。
#[tokio::test]
async fn id_and_dir_are_either_both_or_neither() {
    let b = bench().await;
    let (services, x_wonton) = plain("app.agent.hello", "0.1.0", "");
    b.write_package("hello", services, &x_wonton);
    let before = b.s.cards.posted.lock().unwrap().len();
    for args in [
        json!({"id": "app.example.journal", "dir": "hello"}),
        json!({}),
    ] {
        let result = b.s.call(INSTALL, args).await;
        assert!(
            failed(&result) && text(&result).contains("invalid_input"),
            "{result:?}"
        );
    }
    assert_eq!(b.s.cards.posted.lock().unwrap().len(), before, "零卡片");
}
