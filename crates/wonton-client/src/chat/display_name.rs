//! 主人在资料里填的名字同步成 Matrix 上的显示名（ADR-0030），Agent 和联系人那边看到的就是它。

use std::sync::Arc;

use wonton_matrix::MatrixSession;

use super::{ChatLink, lock};

impl ChatLink {
    /// 改了资料后调用。还没登录 Matrix 时先记着，登录后再设。
    pub(crate) fn set_owner_display_name(&self, name: String) {
        let session = lock(&self.session).clone();
        match session {
            Some(session) => self.spawn_display_name(&session, name),
            None => *lock(&self.display_name) = Some(name),
        }
    }

    pub(super) fn apply_pending_display_name(&self, session: &Arc<MatrixSession>) {
        if let Some(name) = lock(&self.display_name).take() {
            self.spawn_display_name(session, name);
        }
    }

    fn spawn_display_name(&self, session: &Arc<MatrixSession>, name: String) {
        let session = session.clone();
        self.spawn_op("同步显示名", async move {
            session.set_display_name(&name).await
        });
    }
}
