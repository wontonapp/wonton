//! 底栏模块共用的小工具。

use std::sync::Mutex;

use wonton_protocol::{CoreError, TabRef};

use super::TabsState;
use super::view::is_plugin_page;
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

pub(crate) fn clear_plugin_config(state: &mut TabsState) {
    state.config.items.retain(|item| !is_plugin_page(&item.tab));
    state.config.displays.clear();
    state.overlay = None;
    state.fullscreen = None;
    if is_plugin_page(&state.selected) {
        state.selected = TabRef::Apps;
    }
}

pub(crate) fn storage_problem(error: &CoreError) -> super::TabsProblem {
    super::TabsProblem {
        message: l10n!(
            CATALOG,
            language(),
            "tabs-storage-problem",
            error = error.to_string()
        ),
    }
}

pub(crate) fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
