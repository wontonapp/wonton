//! 假数据入口（`ui_fake.rs`）里“等你确认”列表的一张卡片：Bot 请主人装一个要串口、USB 和硬盘的应用（ADR-0036）。
//!
//! 设备项照核心预览的样子给：一个服务要的串口有两个可选，硬盘两块、一块是系统盘，都没选中。回答这张卡片时
//! 没有核心可发：卡片挪到“结束的”里，同意的结果写的就是随回答带上的设备 ID，界面测试据此看壳发了什么。
//!
//! 卡片随开出的那一轮认在这条链上：链停了就撤回（ADR-0061 第 10 节），壳才看得到撤卡这一档。

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

/// 停链撤回的说明，和核心英文文案一致。
const WITHDRAWN: &str = "The chain was stopped; this card is void";

/// 链上开了一轮：把还在等的卡片挂到这条链上，停链时按它撤回。
pub(super) fn bind_chain(view: &mut ConfirmationsViewModel, chain: &str) -> bool {
    let mut changed = false;
    for card in &mut view.pending {
        if card.card.id == CARD_ID {
            card.card.id = format!("{CARD_ID}@{chain}");
            changed = true;
        }
    }
    changed
}

/// 停链撤掉这条链上还等着的卡片：和核心一样落成 `Failed`，按钮收走，挪进“结束的”。
pub(super) fn withdraw(view: &mut ConfirmationsViewModel, chain: &str) -> bool {
    let suffix = format!("@{chain}");
    let (withdrawn, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut view.pending)
        .into_iter()
        .partition(|card| card.card.id.ends_with(&suffix));
    view.pending = pending;
    if withdrawn.is_empty() {
        return false;
    }
    for mut card in withdrawn {
        card.card.choices.clear();
        card.card.state = ConfirmationState::Failed {
            message: WITHDRAWN.into(),
        };
        view.finished.insert(0, card);
    }
    true
}

/// 卡片挂在哪条链上。
fn chain_of(id: &str) -> Option<&str> {
    id.strip_prefix(&format!("{CARD_ID}@"))
}

/// 回答假卡片：同意的结果是带上的设备 ID（一行一个，没带时为空），别的回答算拒绝。
/// 卡片所在的链已经停了时，回答落成撤回（`Failed`），不是同意。不是这张卡片时不动。
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
    answered.card.state = match chain_of(&answered.card.id) {
        Some(chain) if super::ui_fake_agent_stop::was_stopped(chain) => ConfirmationState::Failed {
            message: WITHDRAWN.into(),
        },
        _ => match answer {
            Answer::Approve | Answer::ApproveWider | Answer::AllowWindow => {
                ConfirmationState::Succeeded {
                    message: devices.join("\n"),
                }
            }
            Answer::Reject | Answer::Unknown => ConfirmationState::Rejected,
        },
    };
    view.finished.insert(0, answered);
    true
}

pub(super) fn card() -> ConfirmationCard {
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
    let kind = if by_id.starts_with("nvme-") {
        "nvme:"
    } else {
        ""
    };
    PluginDevice {
        id: format!("disk:{kind}/dev/disk/by-id/{by_id}"),
        name: name.into(),
        target: if by_id.starts_with("nvme-") {
            "/dev/nvme0n1"
        } else {
            "/dev/sda"
        }
        .into(),
        service: "smart".into(),
        optional: false,
        kind: PluginDeviceKind::Disk,
        system,
        chosen: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> ConfirmationsViewModel {
        confirmations(ConfirmationsViewModel::default())
    }

    #[test]
    fn a_stopped_chain_withdraws_the_card() {
        let mut view = seeded();
        let chain = "ui-fake-test-stopped:hello";
        assert!(bind_chain(&mut view, chain));
        assert!(withdraw(&mut view, chain));
        assert!(view.pending.is_empty());
        let card = &view.finished[0].card;
        assert!(matches!(card.state, ConfirmationState::Failed { .. }));
        assert!(card.choices.is_empty(), "撤回的卡片不能再点");
    }

    #[test]
    fn another_chain_leaves_the_card_alone() {
        let mut view = seeded();
        assert!(bind_chain(&mut view, "ui-fake-test-mine:a"));
        assert!(!withdraw(&mut view, "ui-fake-test-other:b"));
        assert_eq!(view.pending.len(), 1);
    }

    #[test]
    fn answering_a_stopped_chain_lands_withdrawn() {
        let mut view = seeded();
        let chain = "ui-fake-test-answered:a";
        super::super::ui_fake_agent_stop::mark_stopped(chain);
        bind_chain(&mut view, chain);
        let id = view.pending[0].card.id.clone();
        assert!(answer(&mut view, &id, Answer::Approve, &["serial".into()]));
        assert!(matches!(
            view.finished[0].card.state,
            ConfirmationState::Failed { .. }
        ));
    }

    #[test]
    fn answering_a_live_card_keeps_the_devices() {
        let mut view = seeded();
        let id = view.pending[0].card.id.clone();
        assert!(answer(&mut view, &id, Answer::Approve, &["serial".into()]));
        assert_eq!(
            view.finished[0].card.state,
            ConfirmationState::Succeeded {
                message: "serial".into()
            }
        );
    }
}
