//! 备份恢复：拒绝创建和挂起读取。

use std::sync::atomic::Ordering;

use super::Home;

impl Home {
    pub fn reject_backup_creation(&self, reject: bool) {
        self.core
            .reject_backup_creation
            .store(reject, Ordering::SeqCst);
    }

    pub fn fail_verification(&self, status: u16) {
        self.core
            .verification_failure
            .store(status as usize, Ordering::SeqCst);
    }

    pub fn hold_backup_read(&self, hold: bool) {
        self.core.hold_backup_read.send_replace(hold);
    }
}
