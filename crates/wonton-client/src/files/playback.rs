//! 卡片上的按钮、音视频能不能在 App 里播、播不了时为什么（ADR-0020）。判断放在这里，两端的壳不各写一份。

use std::path::Path;

use reqwest::Method;
use reqwest::header::RANGE;
use wonton_l10n::l10n;
use wonton_protocol::{
    FileCardAction, FileCardActions, FileFailure, FileId, FileInfo, FileOperation, FileProblem,
    TransferDirection, TransferState,
};

use super::FilesLink;
use super::api::{Api, Problem, checked, interrupted};
use super::download::relative_path;
use crate::l10n::{CATALOG, language};

/// 问核心的期限：播放器已经放不了了，不能让主人再等很久。
const PROBE: std::time::Duration = std::time::Duration::from_secs(30);

/// 两端的播放器都放得了的类型，取自核心写真实类型的媒体（`wonton-core` 的 `files/content.rs`）：别的类型核心按附件给，播放器认不出来。
const PLAYABLE: &[&str] = &[
    "audio/mpeg",
    "audio/mp4",
    "audio/aac",
    "audio/flac",
    "audio/wav",
    "video/mp4",
    "video/quicktime",
];
/// AVPlayer 放不了，Android 的 `MediaPlayer` 放得了。
const PLAYABLE_ON_ANDROID: &[&str] = &["audio/ogg", "audio/opus", "video/webm", "video/x-matroska"];
/// 没声明类型，或者声明的类型不在上面时看扩展名，和核心写类型的办法一样。
const BY_EXTENSION: &[(&str, &str)] = &[
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

fn plays(mime: &str, android: bool) -> bool {
    PLAYABLE.contains(&mime) || (android && PLAYABLE_ON_ANDROID.contains(&mime))
}

fn playable_on(info: &FileInfo, android: bool) -> bool {
    let declared = info
        .mime_type
        .as_deref()
        .and_then(|mime| mime.split(';').next())
        .map(|mime| mime.trim().to_ascii_lowercase());
    if declared.is_some_and(|mime| plays(&mime, android)) {
        return true;
    }
    let extension = Path::new(&info.name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    extension.is_some_and(|extension| {
        BY_EXTENSION
            .iter()
            .any(|(known, mime)| *known == extension && plays(mime, android))
    })
}

fn playable(info: &FileInfo) -> bool {
    playable_on(info, !cfg!(target_os = "ios"))
}

impl FilesLink {
    pub(crate) fn card_actions(&self, info: &FileInfo) -> FileCardActions {
        let view = self.store.view();
        let download = view
            .all()
            .find(|transfer| {
                transfer.direction == TransferDirection::Download
                    && transfer.file.as_ref() == Some(&info.id)
            })
            .cloned();
        let busy = download.as_ref().is_some_and(|download| {
            matches!(
                download.state,
                TransferState::Running | TransferState::Paused
            )
        });
        // 记录清掉了文件也可能还在：主人选了只清记录。
        let path = self.downloads_dir.join(relative_path(&info.id, &info.name));
        let fetch = if busy {
            FileCardAction::Wait
        } else if path.is_file() {
            FileCardAction::Open {
                path: path.to_string_lossy().into_owned(),
            }
        } else {
            FileCardAction::Download
        };
        if playable(info) {
            FileCardActions {
                tap: FileCardAction::Play,
                open_elsewhere: Some(fetch),
                download,
            }
        } else {
            FileCardActions {
                tap: fetch,
                open_elsewhere: None,
                download,
            }
        }
    }

    pub(crate) async fn playback_problem(&self, id: FileId) -> FileProblem {
        let (reason, detail) = match self.stream.take_interruption(&id) {
            Some(detail) => (FileFailure::TransportUnavailable, detail),
            None => self.probe(&id).await,
        };
        FileProblem {
            operation: FileOperation::Play,
            file: id,
            reason,
            message: explain(reason, &detail),
        }
    }

    /// 播放的时候没断过，就再问一次核心。给得出内容，放不了的就是格式。
    async fn probe(&self, id: &FileId) -> (FileFailure, String) {
        let asked = async {
            let api = Api::connect(&self.home)?;
            let response = api
                .stream(Method::GET, &Api::content_path(id))
                .header(RANGE, "bytes=0-0")
                .send()
                .await
                .map_err(interrupted)?;
            checked(response).await
        };
        match tokio::time::timeout(PROBE, asked).await {
            Ok(Ok(_)) => (FileFailure::UnsupportedFormat, String::new()),
            Ok(Err(problem)) => problem.into_parts(),
            Err(_) => {
                Problem::Interrupted(l10n!(CATALOG, language(), "files-playback-no-response"))
                    .into_parts()
            }
        }
    }
}

fn explain(reason: FileFailure, detail: &str) -> String {
    match reason {
        // 断在哪一步对主人没用，连接状态由标题栏说。
        FileFailure::TransportUnavailable => {
            l10n!(CATALOG, language(), "files-playback-transport")
        }
        FileFailure::UnsupportedFormat => {
            l10n!(CATALOG, language(), "files-playback-unsupported")
        }
        // 文件拿不到，下载也一样，不提示。
        FileFailure::Unauthenticated
        | FileFailure::NotShared
        | FileFailure::FileUnknown
        | FileFailure::FileGone => l10n!(
            CATALOG,
            language(),
            "files-playback-unavailable",
            detail = detail
        ),
        _ => l10n!(CATALOG, language(), "files-playback-error", detail = detail),
    }
}

#[cfg(test)]
mod tests {
    use wonton_protocol::FileSource;

    use super::*;

    fn info(name: &str, mime_type: Option<&str>) -> FileInfo {
        FileInfo {
            id: "aaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            name: name.to_owned(),
            mime_type: mime_type.map(str::to_owned),
            size: None,
            source: FileSource::Device,
            has_thumbnail: false,
        }
    }

    #[test]
    fn the_phone_decides_which_formats_play() {
        for (name, mime, ios, android) in [
            ("a.mp4", Some("video/mp4"), true, true),
            ("a.bin", Some("video/mp4; codecs=avc1"), true, true),
            ("a.MP3", None, true, true),
            ("a.mp3", Some("application/octet-stream"), true, true),
            ("a.ogg", Some("audio/ogg"), false, true),
            ("a.mkv", None, false, true),
            ("a.webm", Some("video/webm"), false, true),
            ("a.pdf", Some("application/pdf"), false, false),
            ("a.jpg", Some("image/jpeg"), false, false),
            ("noextension", None, false, false),
        ] {
            let file = info(name, mime);
            assert_eq!(playable_on(&file, false), ios, "iOS {name}");
            assert_eq!(playable_on(&file, true), android, "Android {name}");
        }
    }
}
