//! 壳让用户挑的两张名单（核心 → 客户端核）：能用哪些媒体源、能共享给谁。
//!
//! [`sources_path`] 只列候选，不替人挑：用哪一家仍由核心按“选哪个提供方”的规则定（`notes/0113-interfaces.md`），
//! 客户端把用户挑的那家作为 `source` 带回去。[`SHARE_TARGETS_PATH`] 是能共享给的人，只有显示名和 Matrix ID，
//! 不带角色和权限；写操作里的人（[`super::library::LibraryAction`] 的 `people`、`share_with`）用的就是这个 Matrix ID。

use serde::{Deserialize, Serialize};

use super::api::MEDIA_API_PREFIX;
use super::provider::{kind_param, segment};
use super::{MediaKind, MediaOperation};

/// `GET`：[`ShareTargets`]。
pub const SHARE_TARGETS_PATH: &str = "/_wonton/api/media/share-targets";

/// `GET /sources?operation=…&kind=…`：回 [`MediaSources`]。
pub fn sources_path(query: &SourcesQuery) -> String {
    let mut path = format!(
        "{MEDIA_API_PREFIX}/sources?operation={}",
        segment(query.operation.wire())
    );
    if let Some(kind) = &query.kind {
        path.push_str(&format!("&kind={}", segment(kind_param(kind))));
    }
    path
}

/// `GET /sources` 的查询参数：要做哪样、对哪一种内容。`kind` 空着不看种类（同上传选提供方）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SourcesQuery {
    pub operation: MediaOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MediaKind>,
}

/// `GET /sources` 的回包：这个人能用、又能做这件事的媒体源。一家都没有是空表，不是失败。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MediaSources {
    pub sources: Vec<MediaSource>,
}

/// 一个媒体源插件。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MediaSource {
    /// 插件 id，选定后作为 `source` 带回核心。
    pub id: String,
    /// 插件名，按请求的语言挑好。
    pub title: String,
    /// 主人在能力设置里给 `media_source` 定的默认
    pub is_default: bool,
}

/// `GET /share-targets` 的回包：能共享给的人。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ShareTargets {
    pub people: Vec<ShareTarget>,
}

/// 能共享给的一个人。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ShareTarget {
    /// Matrix 用户 ID。
    pub user_id: String,
    /// 没填时为空，壳显示 `user_id`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sources_path_carries_operation_and_kind() {
        let library = SourcesQuery {
            operation: MediaOperation::Library,
            kind: Some(MediaKind::Photo),
        };
        assert_eq!(
            sources_path(&library),
            "/_wonton/api/media/sources?operation=library&kind=photo"
        );
        let upload = SourcesQuery {
            operation: MediaOperation::Upload,
            kind: None,
        };
        assert_eq!(
            sources_path(&upload),
            "/_wonton/api/media/sources?operation=upload"
        );
        let unknown = SourcesQuery {
            operation: MediaOperation::Other("a&b".into()),
            kind: Some(MediaKind::Other("3d model".into())),
        };
        assert_eq!(
            sources_path(&unknown),
            "/_wonton/api/media/sources?operation=a%26b&kind=3d%20model"
        );
        assert_eq!(SHARE_TARGETS_PATH, "/_wonton/api/media/share-targets");
    }

    #[test]
    fn sources_query_reads_known_and_unknown_values() {
        let query: SourcesQuery =
            serde_json::from_value(json!({"operation": "library", "kind": "photo"})).unwrap();
        assert_eq!(query.operation, MediaOperation::Library);
        assert_eq!(query.kind, Some(MediaKind::Photo));

        let query: SourcesQuery =
            serde_json::from_value(json!({"operation": "transcode", "kind": "hologram"})).unwrap();
        assert_eq!(query.operation, MediaOperation::Other("transcode".into()));
        assert_eq!(query.kind, Some(MediaKind::Other("hologram".into())));

        let query: SourcesQuery = serde_json::from_value(json!({"operation": "upload"})).unwrap();
        assert_eq!(query.kind, None);
    }

    #[test]
    fn sources_wire_shape() {
        let sources = MediaSources {
            sources: vec![MediaSource {
                id: "app.example.photos".into(),
                title: "Photos".into(),
                is_default: true,
            }],
        };
        let wire = json!({"sources": [
            {"id": "app.example.photos", "title": "Photos", "is_default": true}
        ]});
        assert_eq!(serde_json::to_value(&sources).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<MediaSources>(wire).unwrap(),
            sources
        );
        let empty: MediaSources = serde_json::from_value(json!({"sources": []})).unwrap();
        assert!(empty.sources.is_empty());
    }

    #[test]
    fn share_targets_wire_shape() {
        let targets = ShareTargets {
            people: vec![
                ShareTarget {
                    user_id: "@mom:example.com".into(),
                    display_name: Some("Mom".into()),
                },
                ShareTarget {
                    user_id: "@owner:example.com".into(),
                    display_name: None,
                },
            ],
        };
        let wire = json!({"people": [
            {"user_id": "@mom:example.com", "display_name": "Mom"},
            {"user_id": "@owner:example.com"}
        ]});
        assert_eq!(serde_json::to_value(&targets).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ShareTargets>(wire).unwrap(),
            targets
        );
    }

    #[test]
    fn newer_core_fields_are_ignored() {
        let target: ShareTarget = serde_json::from_value(
            json!({"user_id": "@kid:example.com", "display_name": "Kid", "avatar": "mxc://x"}),
        )
        .unwrap();
        assert_eq!(target.user_id, "@kid:example.com");
        let source: MediaSource = serde_json::from_value(
            json!({"id": "a", "title": "A", "is_default": false, "icon": "x"}),
        )
        .unwrap();
        assert!(!source.is_default);
    }
}
