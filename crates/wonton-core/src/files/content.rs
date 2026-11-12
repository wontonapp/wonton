//! 下载的响应怎么写：类型、`Range`。文件内容不可信，核心地址上不能渲染出它的 HTML（ADR-0020 的“接口”）。

/// 可以写真实类型的媒体：图片、音频、视频，都不会执行脚本。不收 `image/svg+xml`：SVG 能带脚本。
const MEDIA: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("avif", "image/avif"),
    ("mp3", "audio/mpeg"),
    ("m4a", "audio/mp4"),
    ("aac", "audio/aac"),
    ("flac", "audio/flac"),
    ("wav", "audio/wav"),
    ("ogg", "audio/ogg"),
    ("opus", "audio/opus"),
    ("mp4", "video/mp4"),
    ("m4v", "video/mp4"),
    ("mov", "video/quicktime"),
    ("webm", "video/webm"),
    ("mkv", "video/x-matroska"),
];

/// 能做缩略图的类型：缩略图子进程解得了的格式。
const THUMBNAIL_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// 插件声明的类型，不带参数。只认没有空白的 ASCII：要写进给 Agent 的文字和给 App 的信息，不能多出一行。
pub fn declared_type(declared: &str) -> Option<&str> {
    let bare = declared.split(';').next()?.trim();
    (!bare.is_empty() && bare.chars().all(|c| c.is_ascii_graphic())).then_some(bare)
}

/// 下载时写的类型：声明的类型或扩展名在 [`MEDIA`] 里才写，否则为空，按附件下载。声明的类型和内容对不上也不要紧：响应带
/// `nosniff`，这些类型都不会被当成网页。
pub fn served_type(name: &str, declared: Option<&str>) -> Option<&'static str> {
    let declared = declared
        .and_then(|d| d.split(';').next())
        .map(|d| d.trim().to_ascii_lowercase());
    let by_declared = declared
        .as_deref()
        .and_then(|d| MEDIA.iter().find(|(_, mime)| *mime == d));
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let by_extension = || {
        let extension = extension.as_deref()?;
        MEDIA.iter().find(|(known, _)| *known == extension)
    };
    by_declared.or_else(by_extension).map(|(_, mime)| *mime)
}

pub fn can_thumbnail(name: &str, declared: Option<&str>) -> bool {
    served_type(name, declared).is_some_and(|mime| THUMBNAIL_TYPES.contains(&mime))
}

/// `view_file` 碰到的媒体类型（ADR-0104）。PDF 只看声明的类型和扩展名，不看内容嗅探。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Pdf,
    /// MP4、M4V、MOV：核心里有解析器。
    Mp4,
    /// webm、mkv：另一个容器，核心里没解析器。
    OtherVideo,
    Audio,
}

/// PDF 看声明的类型或扩展名；视频和音频按 [`served_type`] 归一后的类型分。
pub fn media_kind(name: &str, declared: Option<&str>) -> Option<MediaKind> {
    let declared = declared
        .and_then(declared_type)
        .map(str::to_ascii_lowercase);
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    if declared.as_deref() == Some("application/pdf") || extension.as_deref() == Some("pdf") {
        return Some(MediaKind::Pdf);
    }
    Some(match served_type(name, declared.as_deref())? {
        "video/mp4" | "video/quicktime" => MediaKind::Mp4,
        "video/webm" | "video/x-matroska" => MediaKind::OtherVideo,
        mime if mime.starts_with("audio/") => MediaKind::Audio,
        _ => return None,
    })
}

/// 这次要发文件的哪一段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Full,
    /// 含两端。
    Part {
        start: u64,
        end: u64,
    },
    /// `Range` 超出了文件：416。
    Unsatisfiable,
}

/// 按 `Range` 和 `If-Range` 选出要发的一段。只支持一段：多段的和读不懂的 `Range` 按规范忽略，发整个文件。`If-Range` 只认和
/// 当前强 `ETag` 一样的；日期也按对不上处理，从头下。
pub fn select(range: Option<&str>, if_range: Option<&str>, etag: &str, size: u64) -> Selection {
    let Some(range) = range else {
        return Selection::Full;
    };
    if if_range.is_some_and(|validator| validator.trim() != etag) {
        return Selection::Full;
    }
    let Some(spec) = range.trim().strip_prefix("bytes=") else {
        return Selection::Full;
    };
    let Some((first, last)) = spec.trim().split_once('-') else {
        return Selection::Full;
    };
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    // 数字大到放不下 u64 时，也一定超出了文件。
    let number = |text: &str| text.parse::<u64>().unwrap_or(u64::MAX);
    match (first, last) {
        ("", suffix) if digits(suffix) => match (number(suffix), size) {
            (0, _) | (_, 0) => Selection::Unsatisfiable,
            (suffix, size) => Selection::Part {
                start: size.saturating_sub(suffix),
                end: size - 1,
            },
        },
        (first, "") if digits(first) => {
            let start = number(first);
            if start >= size {
                Selection::Unsatisfiable
            } else {
                Selection::Part {
                    start,
                    end: size - 1,
                }
            }
        }
        (first, last) if digits(first) && digits(last) => {
            let (start, end) = (number(first), number(last));
            if start > end {
                Selection::Full
            } else if start >= size {
                Selection::Unsatisfiable
            } else {
                Selection::Part {
                    start,
                    end: end.min(size - 1),
                }
            }
        }
        _ => Selection::Full,
    }
}

/// `Content-Disposition` 的值：文件名按 RFC 5987 编码，不可信的名字进不了别的响应头。
pub fn attachment(name: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    format!(
        "attachment; filename*=UTF-8''{}",
        utf8_percent_encode(name, NON_ALPHANUMERIC)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: &str = "\"1-2-3\"";

    fn part(start: u64, end: u64) -> Selection {
        Selection::Part { start, end }
    }

    #[test]
    fn ranges_select_one_part() {
        let cases = [
            ("bytes=0-9", part(0, 9)),
            ("bytes=90-", part(90, 99)),
            ("bytes=-10", part(90, 99)),
            ("bytes=-1000", part(0, 99)),
            ("bytes=90-1000", part(90, 99)),
            (" bytes=1-1 ", part(1, 1)),
        ];
        for (range, expected) in cases {
            assert_eq!(select(Some(range), None, TAG, 100), expected, "{range}");
        }
    }

    #[test]
    fn ranges_past_the_end_are_refused() {
        for range in [
            "bytes=100-",
            "bytes=100-200",
            "bytes=-0",
            "bytes=99999999999999999999999-",
        ] {
            let selected = select(Some(range), None, TAG, 100);
            assert_eq!(selected, Selection::Unsatisfiable, "{range}");
        }
        assert_eq!(
            select(Some("bytes=0-"), None, TAG, 0),
            Selection::Unsatisfiable
        );
    }

    #[test]
    fn ranges_we_do_not_support_send_the_whole_file() {
        for range in [
            "bytes=0-1,3-4",
            "items=0-1",
            "bytes=a-b",
            "bytes=5-1",
            "bytes",
        ] {
            assert_eq!(
                select(Some(range), None, TAG, 100),
                Selection::Full,
                "{range}"
            );
        }
        assert_eq!(select(None, Some(TAG), TAG, 100), Selection::Full);
    }

    #[test]
    fn if_range_only_matches_the_current_strong_etag() {
        let range = Some("bytes=10-");
        assert_eq!(select(range, Some(TAG), TAG, 100), part(10, 99));
        for stale in ["\"other\"", "W/\"1-2-3\"", "Wed, 21 Oct 2015 07:28:00 GMT"] {
            assert_eq!(select(range, Some(stale), TAG, 100), Selection::Full);
        }
    }

    #[test]
    fn only_media_keeps_its_type() {
        assert_eq!(served_type("a.JPG", None), Some("image/jpeg"));
        assert_eq!(served_type("a", Some("image/png; x=y")), Some("image/png"));
        assert_eq!(served_type("a.mp4", Some("text/html")), Some("video/mp4"));
        for (name, declared) in [
            ("a.html", Some("text/html")),
            ("a.svg", Some("image/svg+xml")),
            ("a.svg", None),
            ("a.pdf", Some("application/pdf")),
            ("noextension", None),
        ] {
            assert_eq!(served_type(name, declared), None, "{name}");
        }
    }

    #[test]
    fn thumbnails_only_for_formats_the_worker_decodes() {
        assert!(can_thumbnail("a.jpg", None));
        assert!(can_thumbnail("x", Some("image/webp")));
        assert!(!can_thumbnail("a.heic", None));
        assert!(!can_thumbnail("a.mp4", None));
        assert!(!can_thumbnail("a.svg", None));
    }

    #[test]
    fn view_kinds_go_by_declared_type_or_extension() {
        assert_eq!(media_kind("a.pdf", None), Some(MediaKind::Pdf));
        assert_eq!(
            media_kind("a", Some("application/pdf")),
            Some(MediaKind::Pdf)
        );
        assert_eq!(
            media_kind("a.PDF", Some("text/plain")),
            Some(MediaKind::Pdf)
        );
        assert_eq!(media_kind("a.mp4", None), Some(MediaKind::Mp4));
        assert_eq!(
            media_kind("a", Some("video/quicktime")),
            Some(MediaKind::Mp4)
        );
        assert_eq!(media_kind("a.webm", None), Some(MediaKind::OtherVideo));
        assert_eq!(media_kind("a.mkv", None), Some(MediaKind::OtherVideo));
        assert_eq!(media_kind("a.mp3", None), Some(MediaKind::Audio));
        assert_eq!(media_kind("a", Some("audio/ogg")), Some(MediaKind::Audio));
        assert_eq!(media_kind("a.svg", None), None);
        assert_eq!(media_kind("a.jpg", None), None);
    }

    #[test]
    fn file_names_cannot_inject_headers() {
        let value = attachment("a\r\nSet-Cookie: x=1;\".txt");
        assert!(value.is_ascii());
        assert!(!value["attachment; ".len()..].contains([' ', '"', ';', '\r', '\n']));
        assert_eq!(
            attachment("照片.jpg"),
            "attachment; filename*=UTF-8''%E7%85%A7%E7%89%87%2Ejpg"
        );
    }
}
