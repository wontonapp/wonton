//! 建相册、共享相册时能选谁：核心给的名单（主人和已加入的家人，不含自己），任何家人都能取，不用“管理成员”权限。

use std::sync::Arc;

use wonton_protocol::media::choices::{SHARE_TARGETS_PATH, ShareTargets};
use wonton_protocol::media::library_view::PhotoShareTarget;

use super::{PhotosLink, Section};
use crate::media_link::MediaApi;

impl PhotosLink {
    /// 写操作表说这个源能建相册或共享相册时才调。
    pub(super) fn share_targets(self: &Arc<Self>) {
        let Some(ticket) = self.start(Section::ShareTargets, |_, view| {
            view.share_targets.loading = true;
            true
        }) else {
            return;
        };
        let api = MediaApi::connect(&self.home);
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result = match api {
                Ok(api) => api.get::<ShareTargets>(SHARE_TARGETS_PATH).await,
                Err(failure) => Err(failure),
            };
            link.apply(&ticket, |_, view| {
                let targets = &mut view.share_targets;
                targets.loading = false;
                match result {
                    Ok(listed) => {
                        targets.people = listed
                            .people
                            .into_iter()
                            .map(PhotoShareTarget::from)
                            .collect();
                        targets.problem = None;
                    }
                    // 名单不确定就不给选：留着旧名单会让人共享给已经不在的家人。
                    Err(failure) => {
                        targets.people.clear();
                        targets.problem = Some(failure);
                    }
                }
            });
        });
    }
}
