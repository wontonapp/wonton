//! 设备档（ADR-0113 第 4 节）：壳报这台设备的播放器能直接放什么，客户端核拼成 [`DeviceProfile`]，起播
//! 时带给核心、核心再交给提供方挑流。
//!
//! 存在这台服务器的槽位目录里：同一台手机连两台服务器（各是一个人）各存一份，换了手机各是各的。
//! 壳没报过或报的是空的就不起播：空数组到了提供方那边等于“什么都放不了”，会把能直放的也转码。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use wonton_protocol::media::{DeviceProfile, MediaFailure, MediaFailureCode, PlaybackRequest};

use super::api::failure;

const FILE: &str = "media-profile.json";

/// 壳报的播放器能力。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlayerSupport {
    /// 能直放的 MIME 类型，带 RFC 6381 的 `codecs` 参数，如 `video/mp4; codecs="avc1,mp4a"`：提供方按里面的
    /// 编解码判断能不能直放，只写容器的那一项等于没说能解什么。
    pub mime_types: Vec<String>,
    /// 能直放的容器，如 `mp4`、`mkv`。
    pub containers: Vec<String>,
    /// 能接的最高码率（bit/s）；0 表示壳不限。
    pub max_bitrate: u32,
    pub hdr: bool,
}

pub(crate) struct Profiles {
    path: PathBuf,
    current: Mutex<Option<DeviceProfile>>,
}

impl Profiles {
    /// 读不出来（没有、坏了）当作还没报过：壳每次启动都会再报一次。
    pub(crate) fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let current = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    /// 记下壳报的能力；去掉空的、重复的，大小写统一。缺了哪一样就不记，回 `unsupported_by_device`。
    pub(crate) fn report(&self, support: PlayerSupport) -> Result<DeviceProfile, MediaFailure> {
        let direct_play = normalized(support.mime_types);
        let container = normalized(support.containers);
        if direct_play.is_empty() || container.is_empty() {
            return Err(failure(
                MediaFailureCode::UnsupportedByDevice,
                "the player reported no direct-play formats or containers",
            ));
        }
        let profile = DeviceProfile {
            direct_play,
            max_bitrate: if support.max_bitrate == 0 {
                u32::MAX
            } else {
                support.max_bitrate
            },
            hdr: support.hdr,
            container,
        };
        let bytes = serde_json::to_vec(&profile).expect("a device profile always serializes");
        if let Err(error) = std::fs::write(&self.path, bytes) {
            tracing::warn!(%error, "device profile not saved to disk; using it for this process only");
        }
        *self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(profile.clone());
        Ok(profile)
    }

    /// 起播请求里没带设备档就用记下的；都没有就不起播。
    pub(crate) fn fill(&self, request: &mut PlaybackRequest) -> Result<(), MediaFailure> {
        if !request.profile.direct_play.is_empty() && !request.profile.container.is_empty() {
            return Ok(());
        }
        let current = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        request.profile = current.ok_or_else(|| {
            failure(
                MediaFailureCode::UnsupportedByDevice,
                "this device has not reported what its player can direct-play",
            )
        })?;
        Ok(())
    }
}

fn normalized(values: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        let value = value.trim().to_ascii_lowercase();
        if !value.is_empty() && !out.contains(&value) {
            out.push(value);
        }
    }
    out
}
