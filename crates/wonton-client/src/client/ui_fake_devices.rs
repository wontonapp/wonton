//! 假数据入口（`ui_fake.rs`）里“等你确认”列表的一张卡片：Bot 请主人装一个要串口、USB 和硬盘的应用（ADR-0036）。
//!
//! 设备项照核心预览的样子给：一个服务要的串口有两个可选，硬盘两块、一块是系统盘，都没选中。回答这张卡片时
//! 没有核心可发：卡片挪到“结束的”里，同意的结果写的就是随回答带上的设备 ID，界面测试据此看壳发了什么。

use wonton_protocol::{
    Answer, CardAudience, CardChoice, CardLine, ConfirmationCard, ConfirmationState,
    ConfirmationView, ConfirmationsViewModel, PermissionLine, PermissionRisk, PluginDevice,
    PluginDeviceKind,
};

const CARD_ID: &str = "ui-fake-devices";

/// “等你确认”列表：只有那张带设备的卡片在等回答。
pub(super) fn confirmations(mut view: ConfirmationsViewModel) -> ConfirmationsViewModel {
    view.pending = vec![ConfirmationView {
        card: card(),
        sending: None,
        sending_choice_id: None,
        problem: None,
        window: None,
    }];
    view.finished.clear();
    view.contact_requests.clear();
    view.loading = false;
    view.problem = None;
    view
}

/// 回答假卡片：同意的结果是带上的设备 ID（一行一个，没带时为空），别的回答算拒绝。不是这张卡片时不动。
pub(super) fn answer(
    view: &mut ConfirmationsViewModel,
    id: &str,
    answer: Answer,
    devices: &[String],
) -> bool {
    let Some(index) = view.pending.iter().position(|card| card.card.id == id) else {
        return false;
    };
    let mut answered = view.pending.remove(index);
    answered.card.choices.clear();
    answered.card.state = match answer {
        Answer::Approve | Answer::ApproveWider | Answer::AllowWindow => {
            ConfirmationState::Succeeded {
                message: devices.join("\n"),
            }
        }
        Answer::Reject | Answer::Unknown => ConfirmationState::Rejected,
    };
    view.finished.insert(0, answered);
    true
}

fn card() -> ConfirmationCard {
    ConfirmationCard {
        id: CARD_ID.into(),
        state: ConfirmationState::Pending,
        title: "Echo wants to install Home Hub".into(),
        lines: vec![CardLine {
            text: "Home Hub can read and write everything on the disks you tick.".into(),
            emphasis: true,
        }],
        choices: vec![
            CardChoice {
                answer: Answer::Approve,
                label: "Install".into(),
                choice_id: None,
            },
            CardChoice {
                answer: Answer::Reject,
                label: "Don't install".into(),
                choice_id: None,
            },
        ],
        expires_at_ms: 0,
        window: None,
        plugin_id: Some("app.example.hub".into()),
        secret_inputs: Vec::new(),
        settings: None,
        permissions: vec![
            permission("devices.serial", PermissionRisk::High),
            permission("devices.usb", PermissionRisk::High),
            permission("devices.disk", PermissionRisk::VeryHigh),
        ],
        devices: vec![
            serial(
                "usb-ITead_Sonoff_Zigbee_3.0-if00-port0",
                "Sonoff Zigbee 3.0",
            ),
            serial("usb-Silicon_Labs_CP2102-if00-port0", "CP2102 USB to UART"),
            disk(
                "nvme-Samsung_SSD_990_PRO_1TB",
                "Samsung SSD 990 PRO 1TB",
                true,
            ),
            disk("ata-WDC_WD40EFRX-68N32N0", "WDC WD40EFRX 4TB", false),
        ],
        requester: None,
        category: None,
        origin: None,
        audience: CardAudience::Keeper,
        answered_by: None,
    }
}

fn permission(name: &str, risk: PermissionRisk) -> PermissionLine {
    PermissionLine {
        permission: name.into(),
        text: name.into(),
        risk: Some(risk),
        notice: None,
        supported: true,
        added: false,
        optional: false,
        enabled: false,
        off_loss: None,
        opened: None,
    }
}

fn serial(by_id: &str, name: &str) -> PluginDevice {
    PluginDevice {
        id: format!("serial:0:/dev/serial/by-id/{by_id}"),
        name: name.into(),
        target: "/dev/ttyZIGBEE".into(),
        service: "zigbee2mqtt".into(),
        optional: false,
        kind: PluginDeviceKind::Serial,
        system: false,
        chosen: false,
    }
}

fn disk(by_id: &str, name: &str, system: bool) -> PluginDevice {
    PluginDevice {
        id: format!("disk:/dev/disk/by-id/{by_id}"),
        name: name.into(),
        target: String::new(),
        service: "smart".into(),
        optional: false,
        kind: PluginDeviceKind::Disk,
        system,
        chosen: false,
    }
}
