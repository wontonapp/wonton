//! 文件服务的客户端（ADR-0020）。

mod opener;

pub use opener::{FileOpenError, FileOpener};
