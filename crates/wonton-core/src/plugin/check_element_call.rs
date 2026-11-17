//! `plugin check` 对已知 widget 应用做能力覆盖检查（ADR-0096 的补充）：清单声明的能力只是
//! “页面请求 ∩ 清单声明 ∩ 核心允许”三方交集的一角，漏声明要到运行时才暴露（通话里的按钮经桥发
//! 事件被拒），装又装得上。check 认得出 Element Call 的页面时，对照内置的所需能力清单，缺的按
//! 提醒列出（不是不合格，退出码 0）。

use std::collections::BTreeSet;

use super::languages::Languages;
use super::manifest::Manifest;
use super::manifest_widget::MatrixWidget;

/// Element Call 实际会请求的全部能力，来自 matrix-rust-sdk 的 `get_element_call_required_permissions`
/// （函数在 `bindings/matrix-sdk-ffi`，crates.io 发布的 matrix-sdk 里没有，取不到只能照抄；事件类
/// 条目对应锁定的 0.19.1，标志位另外算上 SDK 经 matrix-widget-api 还要的 MSC4407 粘性事件、
/// `m.always_on_screen` 和 MSC4533 的两个——0.19.1 的清单比 Element Call 旧）。状态事件的
/// state_key 由页面在运行时拼上自己的用户和装置 ID，清单只声明事件类型。SDK 升级时要跟着对，
/// `tests/check.rs` 锁定了 matrix-sdk 的版本号，对不上就红。
pub(crate) const ELEMENT_CALL_CAPABILITIES: &[&str] = &[
    // 读和发的消息类事件。
    "org.matrix.msc2762.receive.event:org.matrix.rageshake_request",
    "org.matrix.msc2762.receive.event:io.element.call.encryption_keys",
    "org.matrix.msc2762.receive.event:io.element.call.reaction",
    "org.matrix.msc2762.receive.event:m.reaction",
    "org.matrix.msc2762.receive.event:m.room.redaction",
    "org.matrix.msc2762.receive.event:m.rtc.decline",
    "org.matrix.msc2762.send.event:org.matrix.rageshake_request",
    "org.matrix.msc2762.send.event:io.element.call.encryption_keys",
    "org.matrix.msc2762.send.event:io.element.call.reaction",
    "org.matrix.msc2762.send.event:m.reaction",
    "org.matrix.msc2762.send.event:m.room.redaction",
    "org.matrix.msc2762.send.event:m.rtc.decline",
    // 只读的状态事件。
    "org.matrix.msc2762.receive.state_event:org.matrix.msc3401.call.member",
    "org.matrix.msc2762.receive.state_event:m.room.name",
    "org.matrix.msc2762.receive.state_event:m.room.member",
    "org.matrix.msc2762.receive.state_event:m.room.encryption",
    "org.matrix.msc2762.receive.state_event:m.room.create",
    // 只发的消息类事件。
    "org.matrix.msc2762.send.event:m.rtc.notification",
    "org.matrix.msc2762.send.event:m.call.notify",
    // 发自己的通话成员状态，state_key 形状见上面的清单注释。
    "org.matrix.msc2762.send.state_event:org.matrix.msc3401.call.member",
    // 给设备的事件。
    "org.matrix.msc3819.receive.to_device:io.element.call.encryption_keys",
    "org.matrix.msc3819.send.to_device:io.element.call.encryption_keys",
    // 标志位。
    "io.element.requires_client",
    "m.always_on_screen",
    "org.matrix.msc4039.download_file",
    "org.matrix.msc4157.send.delayed_event",
    "org.matrix.msc4157.update_delayed_event",
    "org.matrix.msc4407.receive.sticky_event",
    "org.matrix.msc4407.send.sticky_event",
    "org.matrix.msc4515.rtc_transports",
    "org.matrix.msc4533.rtc_livekit_delegate_delayed_leave",
    "org.matrix.msc4533.rtc_livekit_get_token",
];

/// 清单的 widget 页面是不是 Element Call：url 查询里同时带着 Element Call 的三个地址参数
/// `widgetId`、`baseUrl`、`intent`（Element Call 的 docs/url-params.md，matrix-rust-sdk 的
/// `ElementCallUrlParams` 是同一组名字；别的 widget 应用不这么传，如 Jitsi 用 `conferenceUrl`）。
/// 认不出来就不提醒。
pub(crate) fn element_call_widget(manifest: &Manifest) -> Option<&MatrixWidget> {
    let widget = manifest.matrix_widget.as_ref()?;
    let query = widget.url.split_once('?')?.1;
    let names: BTreeSet<&str> = query
        .split('&')
        .map(|pair| pair.split('=').next().unwrap_or_default())
        .collect();
    ["widgetId", "baseUrl", "intent"]
        .into_iter()
        .all(|name| names.contains(name))
        .then_some(widget)
}

/// 缺的能力按提醒列出。文字走目录（四种语言），check 的默认语言是英文。
pub(crate) fn check(package_manifest: &Manifest, report: &mut super::check::Report) {
    let Some(widget) = element_call_widget(package_manifest) else {
        return;
    };
    let declared: BTreeSet<&str> = widget.capabilities.iter().map(String::as_str).collect();
    for capability in ELEMENT_CALL_CAPABILITIES {
        if !declared.contains(capability) {
            report.warnings.push(crate::l10n::text(
                &Languages::default(),
                "plugin-check-element-call-capability-missing",
                &[("capability", (*capability).into())],
            ));
        }
    }
}
