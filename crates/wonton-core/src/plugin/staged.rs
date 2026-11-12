//! 上传之后、主人确认之前暂存的插件包（ADR-0031）：预览给出 `upload:sha256:<摘要>`，确认时带回，App 不用再传一次，
//! 上传的包和按来源取的包就走同一条安装路径。只放内存：确认页一般开不了多久，核心重启后重选文件就行。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::package::PluginPackage;

/// 上传的插件包的来源写法的开头，后面是插件包摘要。
pub const SCHEME: &str = "upload:";
const KEEP: Duration = Duration::from_secs(3600);
/// 一个包最多 2 MiB，暂存的总量因此有上限。
const MAX_STAGED: usize = 8;

struct Staged {
    digest: String,
    at: Instant,
    package: PluginPackage,
}

#[derive(Default)]
pub struct StagedUploads {
    staged: Mutex<VecDeque<Staged>>,
}

impl StagedUploads {
    /// 暂存，返回它的来源。同一个包再传一次时重新计时。满了先丢最早的。
    pub fn stage(&self, package: PluginPackage) -> String {
        let digest = package.digest();
        let source = format!("{SCHEME}{digest}");
        let mut staged = self.staged.lock().expect("暂存表锁中毒");
        staged.retain(|s| s.digest != digest && s.at.elapsed() < KEEP);
        if staged.len() >= MAX_STAGED {
            staged.pop_front();
        }
        staged.push_back(Staged {
            digest,
            at: Instant::now(),
            package,
        });
        source
    }

    /// 暂存的包；过期了或者被挤掉了时为空。`digest` 是来源里 `upload:` 后面的部分。
    pub fn get(&self, digest: &str) -> Option<PluginPackage> {
        let staged = self.staged.lock().expect("暂存表锁中毒");
        staged
            .iter()
            .find(|s| s.digest == digest && s.at.elapsed() < KEEP)
            .map(|s| s.package.clone())
    }

    pub fn remove(&self, digest: &str) {
        let mut staged = self.staged.lock().expect("暂存表锁中毒");
        staged.retain(|s| s.digest != digest);
    }
}
