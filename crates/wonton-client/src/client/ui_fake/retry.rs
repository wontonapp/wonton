//! 假数据下出错页的“重试”：照真核心的样子先进行中一会儿，再给“还是不行”，模拟器里看得到转圈和防连点。

use std::time::Duration;

use wonton_protocol::{ActionFailure, ActionRun, PagesViewModel, PlacedTab, TabPage};

use super::super::ClientCore;

/// 进行中留多久：来得及看到转圈、再点一下确认点不动。
const RUNNING_SHOWN: Duration = Duration::from_secs(2);

impl ClientCore {
    pub(super) fn retry_ui_fake(&self, page: PlacedTab) {
        let pages = self.views().pages.clone();
        let mut started = false;
        pages.send_if_modified(|view| {
            let Some(target) = page_mut(view, &page) else {
                return false;
            };
            if target.retry.is_running() {
                return false;
            }
            target.retry = ActionRun::Running;
            started = true;
            true
        });
        if !started {
            return;
        }
        // 假数据入口没有作用域的运行时可借，结果另起线程晚一点给。
        std::thread::spawn(move || {
            std::thread::sleep(RUNNING_SHOWN);
            pages.send_modify(|view| {
                if let Some(target) = page_mut(view, &page) {
                    target.retry = ActionRun::Failed {
                        failure: ActionFailure::StillFailing,
                        details: None,
                    };
                }
            });
        });
    }
}

fn page_mut<'a>(view: &'a mut PagesViewModel, page: &PlacedTab) -> Option<&'a mut TabPage> {
    match page {
        PlacedTab::Page { tab } | PlacedTab::Content { tab } => view
            .stack
            .iter_mut()
            .find(|open| open.tab == *tab)
            .map(|open| &mut open.page),
        PlacedTab::Panel { .. } => None,
    }
}
