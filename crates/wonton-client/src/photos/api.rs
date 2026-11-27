//! 相册请求的地址：核心的 `/_wonton/api/media` 加提供方路径（`wonton_protocol::media::library` 拼好、转义好的），
//! 再带上选定的相册插件（`source`）和翻页游标。请求本身走 [`MediaApi`](crate::media_link::MediaApi)：设备头、
//! 失败折成 [`MediaFailure`](wonton_protocol::media::MediaFailure) 信封都和其他媒体请求同一套。

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use wonton_protocol::media::api::MEDIA_API_PREFIX;

/// 查询串的值里留原样的字符，同 `library` 的路径拼法（RFC 3986 的 unreserved）。
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// 提供方路径（可能已带查询串）后面接上有值的参数。
pub(super) fn with_query(mut path: String, pairs: &[(&str, Option<&str>)]) -> String {
    for (name, value) in pairs {
        let Some(value) = value else { continue };
        path.push(if path.contains('?') { '&' } else { '?' });
        path.push_str(name);
        path.push('=');
        path.extend(utf8_percent_encode(value, UNRESERVED));
    }
    path
}

/// 核心上的路径，`source` 有值就带上。
pub(super) fn core_path(provider_path: String, source: Option<&str>) -> String {
    format!(
        "{MEDIA_API_PREFIX}{}",
        with_query(provider_path, &[("source", source)])
    )
}

#[cfg(test)]
mod tests {
    use wonton_protocol::media::library::{
        Rendition, SearchQuery, library_content_path, timeline_bucket_path,
    };

    use super::*;

    #[test]
    fn source_and_cursor_follow_whatever_query_the_path_already_has() {
        let month = with_query(timeline_bucket_path("2026-11"), &[("cursor", Some("a b"))]);
        assert_eq!(
            core_path(month, Some("app.example.photos")),
            "/_wonton/api/media/library/timeline/2026-11?cursor=a%20b&source=app.example.photos"
        );
        let bytes = library_content_path("a1", &Rendition::Thumbnail);
        assert_eq!(
            core_path(bytes, None),
            "/_wonton/api/media/library/items/a1/content?rendition=thumbnail"
        );
        let search = SearchQuery::default().path();
        assert_eq!(
            core_path(search, Some("x&y")),
            "/_wonton/api/media/library/search?source=x%26y"
        );
    }
}
