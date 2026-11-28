//! 文件的行为测试（ADR-0020）：只经由导出给原生壳的接口驱动，断言传输列表和核心收到的请求。隧道服务端和假的核心文件接口（含 tus）都跑在
//! 本机。真实的 tusd 和核心的行为见 `wonton-core` 的 `gateway::files_upload_tests`。

mod cards;
mod download;
mod download_resume;
mod operations;
mod transfers;
mod upload;
mod upload_resume;

use std::fs::File;
use std::os::fd::IntoRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::files_support::{Served, World, id, read_opened, source_bytes};
use wonton_client::{
    Action, CoreError, FileCard, FileFailure, FileOpenError, FileOpener, FileOperation, FileSource,
    TransferDirection, TransferState, UploadSource,
};

pub(super) const MIB: usize = 1024 * 1024;

pub(super) fn path_source(path: &std::path::Path) -> UploadSource {
    UploadSource::Path {
        path: path.to_str().unwrap().to_owned(),
    }
}

pub(super) fn upload(source: UploadSource, name: &str) -> Action {
    Action::Files {
        action: wonton_protocol::FileAction::UploadFile {
            source,
            name: name.to_owned(),
            mime_type: Some("video/mp4".to_owned()),
            room: None,
        },
    }
}

/// 发进房间的上传：传完由核心授权并把链接发进这个房间。
pub(super) fn upload_with_room(source: UploadSource, name: &str) -> Action {
    Action::Files {
        action: wonton_protocol::FileAction::UploadFile {
            source,
            name: name.to_owned(),
            mime_type: Some("video/mp4".to_owned()),
            room: Some("!bot:home".to_owned()),
        },
    }
}

fn failed_with(state: &TransferState, expected: FileFailure) {
    match state {
        TransferState::Failed { reason, .. } => assert_eq!(*reason, expected, "{state:?}"),
        other => panic!("应该失败：{other:?}"),
    }
}
