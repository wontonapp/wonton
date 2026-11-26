use super::{InstallError, Languages};

pub(in crate::plugin) fn unavailable(message: impl Into<String>) -> InstallError {
    InstallError::ModelsUnavailable(message.into())
}

pub(crate) fn gateway_not_running(langs: &Languages, id: &str) -> String {
    crate::l10n::text(
        langs,
        "plugin-settings-models-gateway-not-running",
        &[("id", id.into())],
    )
}

pub(crate) fn gateway_absent(langs: &Languages) -> String {
    crate::l10n::text(langs, "plugin-settings-models-gateway-absent", &[])
}

pub(in crate::plugin) fn reconcile_failed(langs: &Languages, error: &str) -> String {
    crate::l10n::text(
        langs,
        "plugin-settings-models-reconcile-failed",
        &[("error", error.into())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文案不能只留下 l10n id：调用方（`watch.rs` 的旧重试路径）把原因放进 Degraded，缺消息时用户看到的是裸 key。
    #[test]
    fn reconcile_failed_keeps_the_error() {
        let text = reconcile_failed(&Languages::default(), "boom");
        assert!(text.contains("boom"), "{text}");
        assert!(!text.contains("plugin-settings-"), "{text}");
    }
}
