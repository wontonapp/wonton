//! 壳读文件、裁图之前要知道的上限，单独放一个文件，`client.rs` 不超过行数上限。

use super::ClientCore;

#[uniffi::export]
impl ClientCore {
    /// 插件的压缩包最大多少字节。壳读文件之前先看大小，太大的不读进内存。
    pub fn max_plugin_archive_bytes(&self) -> u64 {
        wonton_protocol::MAX_PLUGIN_ARCHIVE_BYTES
    }

    /// 头像的边长上限（像素）。壳按它把主人选的图裁成方形、缩放。
    pub fn max_avatar_edge(&self) -> u32 {
        wonton_protocol::MAX_AVATAR_EDGE
    }

    /// 单个头像文件的大小上限（字节）。壳读文件之前先看大小，太大的不读进内存。
    pub fn max_avatar_bytes(&self) -> u64 {
        wonton_protocol::MAX_AVATAR_BYTES
    }

    /// 动图和视频头像的时长上限（毫秒）。壳按它截取。
    pub fn max_avatar_duration_ms(&self) -> u32 {
        wonton_protocol::MAX_AVATAR_DURATION_MS
    }
}
