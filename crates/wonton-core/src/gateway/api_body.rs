//! 插件管理接口的请求体：JSON 按来源预览或安装，其余当上传的插件包（压缩包按内容认格式，或 compose 文本）。

use hyper::header::CONTENT_TYPE;
use serde::de::DeserializeOwned;

use super::api::{languages, text1};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::package::PluginPackage;
use crate::plugin::{InstallError, package_archive};

/// 上传插件包的请求体上限：压缩包里可以带着插件包以外的文件。插件包本身的大小由解析时检查。
pub(super) const MAX_UPLOAD_BYTES: usize = package_archive::MAX_ARCHIVE_BYTES;

/// `gateway-invalid-body`：JSON 请求体解析不了。
pub(super) fn invalid_body(
    headers: &hyper::HeaderMap,
    error: impl std::fmt::Display,
) -> InstallError {
    InstallError::Invalid(text1(
        &languages(headers),
        "gateway-invalid-body",
        &error.to_string(),
    ))
}

/// JSON 请求体是按来源预览或安装（`InstallSource`、`InstallRequest`）；其余是上传的插件包。解析不了时是
/// `InstallError::Invalid`。
pub(super) fn json_body<T: DeserializeOwned>(
    headers: &hyper::HeaderMap,
    body: &[u8],
) -> Option<Result<T, InstallError>> {
    let json = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/json"));
    json.then(|| serde_json::from_slice(body).map_err(|e| invalid_body(headers, e)))
}

/// 请求体里的插件包：压缩包（按内容认格式，见 [`package_archive`]），或者 compose 文本。
pub(super) fn package(
    langs: &Languages,
    headers: &hyper::HeaderMap,
    body: &[u8],
) -> Result<PluginPackage, InstallError> {
    let tar = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/x-tar"));
    if tar || package_archive::is_archive(body) {
        return Ok(PluginPackage::from_archive(body, langs)?);
    }
    let source = std::str::from_utf8(body)
        .map_err(|_| InstallError::Invalid(l10n::text(langs, "gateway-invalid-package", &[])))?;
    Ok(PluginPackage::parse(source, langs)?)
}
