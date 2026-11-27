//! 核心与提供方之间的上传线上形状（ADR-0113 的“媒体源契约”与第 9 节）：开始、续写、收尾和去重检查。
//!
//! 续写的偏移走 `Upload-Offset` 头，字节走请求体，这里只有 JSON 的那几样。

use serde::{Deserialize, Serialize};

use super::Timestamp;

/// `POST`：建备份任务前问提供方哪些校验和已经在这个人的库里。
pub const PATH_UPLOADS_CHECK: &str = "/uploads/check";

/// `HEAD` 回、`PATCH` 带的已收偏移头（tus 的写法）。
pub const UPLOAD_OFFSET_HEADER: &str = "Upload-Offset";

/// 校验和的写法是“算法:小写 hex”；相册备份用 SHA-1，和 Immich 的去重一致。
pub const CHECKSUM_SHA1_PREFIX: &str = "sha1:";

/// 收尾回 `item_id` 的路径。
pub fn upload_finish_path(upload_id: &str) -> String {
    format!("{}/{upload_id}/finish", super::PATH_UPLOADS)
}

/// 一条上传的路径（`HEAD` 查偏移、`PATCH` 续写、`DELETE` 放弃）。
pub fn upload_path(upload_id: &str) -> String {
    format!("{}/{upload_id}", super::PATH_UPLOADS)
}

/// `POST /uploads` 的体，由核心按任务里的 `UploadItem` 补出来。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UploadStart {
    /// 手机上的相对路径，提供方拿文件名
    pub path: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// 拍摄时间。相册类提供方按它排时间线，没有就回 `protocol` 拒收，不拿上传时刻顶上；
    /// 类型上留可省，给不按时间排的提供方。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_at: Option<Timestamp>,
    /// 实况/动态照片的静态那一项带上动态那段的校验和；动态那段在同一任务里排在前面、先传完
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_video_checksum: Option<String>,
}

/// `POST /uploads` 的回包：上传号和已收字节（新建为 0，断点处续）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UploadStarted {
    pub upload_id: String,
    pub upload_offset: u64,
}

/// `POST /uploads/{id}/finish` 的回包。`duplicate` 为真是收尾时才发现库里已有同一张（检查和上传
/// 之间别处传了），核心把这一项记成跳过，不算失败。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UploadFinished {
    pub item_id: String,
    #[serde(default)]
    pub duplicate: bool,
}

/// `POST /uploads/check` 的体。一次最多 1000 个，多的分批问。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UploadCheck {
    pub checksums: Vec<String>,
}

/// 单次检查的上限。
pub const UPLOAD_CHECK_MAX: usize = 1000;

/// `POST /uploads/check` 的回包：只列已在库里的，不在的不列。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UploadCheckResult {
    pub existing: Vec<ExistingItem>,
}

/// 已在库里的一条。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ExistingItem {
    pub checksum: String,
    pub item_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_paths_join_expected() {
        assert_eq!(PATH_UPLOADS_CHECK, "/uploads/check");
        assert_eq!(upload_path("u1"), "/uploads/u1");
        assert_eq!(upload_finish_path("u1"), "/uploads/u1/finish");
        assert_eq!(UPLOAD_OFFSET_HEADER, "Upload-Offset");
    }

    #[test]
    fn upload_start_omits_unset_and_roundtrips() {
        let start = UploadStart {
            path: "DCIM/IMG_0001.HEIC".into(),
            size: 2048,
            checksum: Some(format!(
                "{CHECKSUM_SHA1_PREFIX}da39a3ee5e6b4b0d3255bfef95601890afd80709"
            )),
            taken_at: Some(1_700_000_000_000),
            live_video_checksum: Some("sha1:0000000000000000000000000000000000000000".into()),
        };
        let back: UploadStart =
            serde_json::from_value(serde_json::to_value(&start).unwrap()).unwrap();
        assert_eq!(back, start);

        let bare: UploadStart =
            serde_json::from_value(serde_json::json!({"path": "a.jpg", "size": 1})).unwrap();
        let json = serde_json::to_value(&bare).unwrap();
        assert_eq!(json, serde_json::json!({"path": "a.jpg", "size": 1}));
    }

    #[test]
    fn finish_defaults_to_not_duplicate() {
        let finished: UploadFinished =
            serde_json::from_value(serde_json::json!({"item_id": "x"})).unwrap();
        assert!(!finished.duplicate);
    }

    #[test]
    fn check_roundtrips() {
        let result = UploadCheckResult {
            existing: vec![ExistingItem {
                checksum: "sha1:aa".into(),
                item_id: "x".into(),
            }],
        };
        let back: UploadCheckResult =
            serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
        assert_eq!(back, result);
        let check: UploadCheck =
            serde_json::from_value(serde_json::json!({"checksums": ["sha1:aa"]})).unwrap();
        assert_eq!(check.checksums.len(), 1);
    }
}
