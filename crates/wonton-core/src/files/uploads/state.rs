//! 一次上传对客户端来说是什么：创建时网关转来的信息，和问进度时的状态。

use wonton_protocol::FileId;

/// 创建上传时 tusd 转来的信息，都是客户端声明的，编号除外（网关定的）。
#[derive(Debug, Clone, Copy)]
pub struct NewUpload<'a> {
    pub id: &'a FileId,
    pub name: Option<&'a str>,
    pub mime_type: Option<&'a str>,
    /// 声明的长度；`Upload-Defer-Length` 时为空。
    pub size: Option<u64>,
    pub device: &'a str,
}

/// 一个编号对上传接口来说是什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadState {
    /// `device` 是创建它的设备，只有它能接着传。
    Pending {
        size: u64,
        device: String,
    },
    Finished {
        size: u64,
    },
    Unknown,
}
