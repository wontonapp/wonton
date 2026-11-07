//! 按 tag 装的社区插件，官方索引后来收录了同一个仓库的同一个提交：认成官方。提交一样，装着的文件就是索引审核过的
//! 那份，不用主人重装；提交对不上的保持社区，要换成官方的那份得主人自己升级、确认。

use std::sync::Arc;

use super::super::github_source::{Origin, SourceRef, Trust};
use super::super::listed::Listed;
use super::super::manager::{PluginManager, busy};
use super::super::manifest::PluginId;
use super::super::store::{InstalledRecord, StoreError};
use super::super::storefront::index::Index;
use crate::plugin::languages::Languages;

impl PluginManager {
    /// 查新版本前，把能认成官方的记录改过来，`records` 跟着改。
    pub(super) fn adopt_official(&self, index: &Index, records: &mut [InstalledRecord]) {
        let mut adopted = false;
        for record in records.iter_mut() {
            let Some((from, to)) = adoptable(index, record) else {
                continue;
            };
            match self.set_origin(&record.id, &from, &to) {
                Ok(true) => {
                    tracing::info!(plugin = %record.id, commit = %to.commit, "官方索引收录了装着的提交，认成官方");
                    record.origin = Some(to);
                    adopted = true;
                }
                Ok(false) => {}
                Err(e) => tracing::warn!(plugin = %record.id, error = %e, "认成官方时写不了记录"),
            }
        }
        if adopted {
            self.changed();
        }
    }

    /// 在插件表的锁里改：和开始升级、卸载互斥，在忙的不碰，它们自己会写记录。记录在这之间换了来源的也不改。
    fn set_origin(&self, id: &PluginId, from: &Origin, to: &Origin) -> Result<bool, StoreError> {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return Ok(false);
        };
        if busy(id, &entry.status, &self.owner_languages()).is_some() {
            return Ok(false);
        }
        let Some(mut record) = self.store.load(id)? else {
            return Ok(false);
        };
        if record.origin.as_ref() != Some(from) {
            return Ok(false);
        }
        record.origin = Some(to.clone());
        self.store.save(&record)?;
        if let Some(listed) = entry.listed.clone() {
            entry.listed = Some(Arc::new(Listed {
                origin: Some(to.clone()),
                ..(*listed).clone()
            }));
        }
        Ok(true)
    }
}

/// 记录现在的来源，和认成官方后的来源。经第三方索引装的只看那个索引（ADR-0031），按提交装的不算。
fn adoptable(index: &Index, record: &InstalledRecord) -> Option<(Origin, Origin)> {
    let origin = record.origin.as_ref()?;
    if origin.trust != Trust::Community || origin.tag.is_none() || origin.index.is_some() {
        return None;
    }
    let listed = index.find(&SourceRef::parse(&origin.repo, &Languages::default()).ok()?)?;
    let same = listed.entry.commit == origin.commit && listed.manifest.id == record.id;
    same.then(|| (origin.clone(), listed.origin()))
}
