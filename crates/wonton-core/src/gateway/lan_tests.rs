//! 局域网状态页的运行状态：局域网里谁都打得开它，只给数字；随核心附带的插件没在运行时逐个写明状态。

use hyper::StatusCode;

use super::tests::Harness;
use crate::plugin::{PluginPackage, PluginStatus};

/// 局域网里的浏览器用服务器的 IP 打开状态页。
const LAN_IP: &str = "192.168.1.20";

fn package(id: &str, title: &str) -> PluginPackage {
    PluginPackage::parse(&format!(
        "services:\n  app:\n    image: test/app\nx-wonton:\n  spec: '0.4'\n  id: {id}\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: {title}, en: {title}}}\n  description: {{zh: x, en: x}}\n  permissions: []\n"
    ))
    .unwrap()
}

#[tokio::test]
async fn the_lan_page_names_bundled_plugins_that_are_not_running_but_not_the_owners_own() {
    let h = Harness::start(None).await;
    let pulling = package("app.agent", "默认助手").manifest;
    h.manager.bundled_for_test(&pulling.id);
    let reason = "预装失败（已试 2 次，稍后自动重试）：拉取镜像失败（test/app）：<超时>";
    let degraded = PluginStatus::Degraded {
        reason: reason.into(),
    };
    h.manager.register_for_test(pulling, degraded, None);
    let installing = package("app.gateway", "模型设置").manifest;
    h.manager.bundled_for_test(&installing.id);
    h.manager
        .register_for_test(installing, PluginStatus::Installing, None);
    let own = package("app.diary", "日记").manifest;
    let broken = PluginStatus::Degraded {
        reason: "日记的数据库坏了".into(),
    };
    h.manager.register_for_test(own, broken, None);

    let page = h.get(LAN_IP, "/", &[]).await;
    assert_eq!(page.status, StatusCode::OK);
    let text = page.text();
    let shown = "默认助手：异常：预装失败（已试 2 次，稍后自动重试）：拉取镜像失败（test/app）：&lt;超时&gt;";
    assert!(text.contains(shown), "{text}");
    assert!(text.contains("模型设置：安装中"), "{text}");
    assert!(text.contains("2 个应用异常"), "{text}");
    assert!(!text.contains("日记"), "主人自己装的插件不列名字：{text}");
}

#[tokio::test]
async fn the_lan_page_leaves_out_the_container_logs_after_a_bundled_plugins_reason() {
    let h = Harness::start(None).await;
    let agent = package("app.agent", "默认助手").manifest;
    h.manager.bundled_for_test(&agent.id);
    let reason = "预装失败：容器 agent 健康检查失败\n最近日志：\n收到主人的消息：明天去医院";
    let unhealthy = PluginStatus::Degraded {
        reason: reason.into(),
    };
    h.manager.register_for_test(agent, unhealthy, None);

    let text = h.get(LAN_IP, "/", &[]).await.text().to_owned();
    assert!(
        text.contains("默认助手：异常：预装失败：容器 agent 健康检查失败"),
        "{text}"
    );
    assert!(!text.contains("医院"), "不给容器日志：{text}");
}
