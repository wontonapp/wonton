//! 底栏模块共用的小工具。

use std::sync::Mutex;

use wonton_protocol::{CoreError, TabRef};

use super::TabsState;
use super::view::is_plugin;

pub(crate) fn clear_plugin_config(state: &mut TabsState) {
    state.config.items.retain(|item| !is_plugin(&item.tab));
    state.config.displays.clear();
    state.overlay = None;
    state.fullscreen = None;
    if is_plugin(&state.selected) {
        state.selected = TabRef::Apps;
    }
}

pub(crate) fn storage_problem(error: &CoreError) -> super::TabsProblem {
    super::TabsProblem {
        message: format!("底栏设置存不下，下次启动可能恢复旧状态：{error}"),
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
