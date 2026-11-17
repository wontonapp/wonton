//! 要管理员批的安装、升级（#767）：必需项里有最高级（`privileged`、`host.pid` 等）时第二张卡照 NVMe
//! 的做法发（ADR-0036）；卡片不把别的来源写成官方索引；成员发起、管理员批准的升级不改可选项原来的
//! 打开人和时间，旧版必需、新版可选的项记装的时候。

use hyper::StatusCode;
use serde_json::{Value, json};

use super::echo_compose;
use super::install_gate_owner_tests::{ask_admin_member, finished, staged};
use super::install_gate_upgrade_tests::required_socket_compose;
use super::install_nvme_gate_tests::{
    answer_admin_card, optional_socket_compose, pending_admin_card, spawn_member_request,
    spawn_member_yaml,
};
use wonton_protocol::{CardLine, PluginTrust};

use crate::plugin::languages::Languages;
use crate::plugin::manifest::Permission;
use crate::plugin::store::OpenedBy;
use crate::plugin::{PluginId, PluginPackage, UpgradeTo};

use super::super::tests::Harness;

const UPGRADE: &str = "/_wonton/api/plugins/app.echo/upgrade";

fn echo() -> PluginId {
    PluginId::parse("app.echo").unwrap()
}

/// 多一个服务用上 `field`，清单的必需项写 `permission`（最高级）。
fn highest_compose(field: &str, permission: &str) -> String {
    let compose = echo_compose()
        .replace(
            "services:\n",
            &format!("services:\n  root: {{image: example/root, {field}}}\n"),
        )
        .replace(
            "  permissions: []",
            &format!("  permissions: [{permission}]"),
        );
    assert!(compose.contains(permission));
    compose
}

fn highest_line(card: &Value, permission: &str) -> bool {
    card["permissions"].as_array().is_some_and(|lines| {
        lines
            .iter()
            .any(|line| line["permission"] == permission && line["risk"] == "highest")
    })
}

/// 卡片里和预览给的这一行相同的行有几行：提示行由预览给出，卡片原样带上。
fn copies(card: &Value, line: &CardLine) -> usize {
    card["lines"].as_array().map_or(0, |lines| {
        lines
            .iter()
            .filter(|item| item["text"] == line.text.as_str())
            .count()
    })
}

fn installed(h: &Harness) -> bool {
    h.manager.store().load(&echo()).unwrap().is_some()
}

/// 成员按来源装一个必需 `privileged` 的上传包：核心认作上传的（版本行不写官方索引），来源只由预览的
/// 未审核提示写一处；第二张卡标出最高级，两张都批了才装，记录里的授予就是这一项必需权限。
#[tokio::test]
async fn a_required_highest_permission_needs_the_second_card() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    let compose = highest_compose("privileged: true", "privileged");
    let package = PluginPackage::parse(&compose, &Languages::default()).unwrap();
    let preview = h
        .manager
        .preview_upload(package, &h.manager.owner_languages())
        .await
        .unwrap();
    assert_eq!(preview.trust, PluginTrust::Local);
    let request = json!({"source": preview.source, "digest": preview.digest, "plan": "",
        "grant": ["privileged"]});
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let first = pending_admin_card(&h).await;
    assert!(!preview.notices.is_empty());
    for notice in &preview.notices {
        assert_eq!(copies(&first, notice), 1, "{first}");
    }
    answer_admin_card(&h, &first, "approve").await;
    assert!(!installed(&h), "第二张没批之前不装");
    let second = pending_admin_card(&h).await;
    assert_ne!(first["id"], second["id"]);
    assert!(highest_line(&second, "privileged"), "{second}");
    answer_admin_card(&h, &second, "approve").await;
    let (status, body) = finished(waiting).await;
    assert!(status.is_success(), "{status} {body}");
    let record = h.manager.store().load(&echo()).unwrap().unwrap();
    assert_eq!(record.granted_permissions, [Permission::Privileged]);
}

/// 成员直接上传一个必需 `host.pid` 的包：第二张卡拒了就什么都没装。
#[tokio::test]
async fn an_uploaded_host_pid_plugin_stops_at_the_second_card() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    let waiting = spawn_member_yaml(
        &h,
        member,
        "/_wonton/api/plugins?grant=host.pid",
        highest_compose("pid: host", "host.pid"),
    );
    let first = pending_admin_card(&h).await;
    answer_admin_card(&h, &first, "approve").await;
    let second = pending_admin_card(&h).await;
    assert_ne!(first["id"], second["id"]);
    assert!(highest_line(&second, "host.pid"), "{second}");
    answer_admin_card(&h, &second, "reject").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!installed(&h));
}

async fn approve_both(h: &Harness, waiting: tokio::task::JoinHandle<(StatusCode, String)>) {
    let first = pending_admin_card(h).await;
    answer_admin_card(h, &first, "approve").await;
    let second = pending_admin_card(h).await;
    assert_ne!(first["id"], second["id"]);
    answer_admin_card(h, &second, "approve").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Bot 经卡片打开的 `docker.socket`：成员原样带着它升级、管理员批准，记录里仍是那个 Bot 和原来的时间，
/// 不记成主人这一刻打开的。
#[tokio::test]
async fn an_approved_member_upgrade_keeps_who_opened_the_permission() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    h.manager
        .install(&optional_socket_compose("0.1.0"), &[], Languages::default())
        .await
        .unwrap();
    let bot = OpenedBy::Bot {
        plugin_id: "com.example.agent".into(),
        title: "Agent".into(),
    };
    h.manager
        .set_plugin_permission(
            echo(),
            Permission::DockerSocket,
            true,
            bot.clone(),
            Languages::default(),
        )
        .await
        .unwrap();
    let before = h.manager.store().load(&echo()).unwrap().unwrap();
    let newer =
        PluginPackage::parse(&optional_socket_compose("0.2.0"), &Languages::default()).unwrap();
    let (source, digest) = staged(&h, newer).await;
    let request = json!({"source": source, "digest": digest, "grant": [],
        "enabled_permissions": ["docker.socket"]});
    approve_both(&h, spawn_member_request(&h, member, UPGRADE, request)).await;
    let record = h.manager.store().load(&echo()).unwrap().unwrap();
    assert_eq!(record.version, "0.2.0");
    assert_eq!(record.enabled_permissions, before.enabled_permissions);
    assert_eq!(record.enabled_permissions[0].opened_by, bot);
}

/// 旧版必需、新版可选的 `docker.socket` 不是这次升级打开的：时间记装的时候，和升级预览画的一致。
#[tokio::test]
async fn a_required_turned_optional_permission_keeps_the_install_time() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    h.manager
        .install(
            &required_socket_compose("0.1.0"),
            &[Permission::DockerSocket],
            Languages::default(),
        )
        .await
        .unwrap();
    let mut record = h.manager.store().load(&echo()).unwrap().unwrap();
    record.installed_at = 1_000;
    h.manager.store().save(&record).unwrap();
    let newer =
        PluginPackage::parse(&optional_socket_compose("0.2.0"), &Languages::default()).unwrap();
    let preview = h
        .manager
        .preview_upgrade(
            &echo(),
            UpgradeTo::Upload(Box::new(newer)),
            None,
            &Languages::default(),
        )
        .await
        .unwrap();
    let request = json!({"source": preview.source, "digest": preview.digest, "grant": [],
        "enabled_permissions": ["docker.socket"]});
    approve_both(&h, spawn_member_request(&h, member, UPGRADE, request)).await;
    let record = h.manager.store().load(&echo()).unwrap().unwrap();
    assert_eq!(record.version, "0.2.0");
    assert_eq!(record.enabled(), [Permission::DockerSocket]);
    assert_eq!(record.enabled_permissions[0].since, 1_000);
    assert!(record.granted_permissions.is_empty());
}
