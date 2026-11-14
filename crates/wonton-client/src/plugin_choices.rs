//! 两端应用商店和角色勾选页共用的展示决定。

use wonton_protocol::{
    Gate, InstallPreview, PluginAudience, PluginAudienceNotice, PluginAudienceView, PluginDetail,
    StoreItem,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PluginInstallAction {
    Install,
    RequestAdmin,
    Blocked,
}

fn install_action(gate: Gate, installable: bool, blocked: bool) -> PluginInstallAction {
    if blocked {
        return PluginInstallAction::Blocked;
    }
    match gate {
        Gate::Allow if installable => PluginInstallAction::Install,
        Gate::AskAdmin => PluginInstallAction::RequestAdmin,
        Gate::Allow | Gate::Deny | Gate::Other => PluginInstallAction::Blocked,
    }
}

#[uniffi::export]
pub fn store_install_action(item: StoreItem) -> PluginInstallAction {
    install_action(item.install_gate, item.installable, item.blocked.is_some())
}

#[uniffi::export]
pub fn detail_install_action(detail: PluginDetail) -> PluginInstallAction {
    install_action(
        detail.install_gate,
        detail.installable,
        detail.blocked.is_some(),
    )
}

#[uniffi::export]
pub fn preview_install_action(preview: InstallPreview) -> PluginInstallAction {
    install_action(preview.install_gate, true, preview.blocked.is_some())
}

#[uniffi::export]
pub fn selected_plugin_audience(view: PluginAudienceView) -> Option<PluginAudience> {
    (!view.choices.is_empty()).then(|| PluginAudience {
        role_ids: view
            .choices
            .into_iter()
            .filter(|choice| choice.required || choice.selected)
            .map(|choice| choice.id)
            .collect(),
    })
}

#[uniffi::export]
pub fn plugin_audience_notice(view: PluginAudienceView) -> Option<PluginAudienceNotice> {
    (view.choices.is_empty()
        || view
            .choices
            .iter()
            .any(|choice| choice.required || choice.selected))
    .then_some(view.notice)
    .flatten()
}
