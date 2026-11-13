//! [`PackageError`](super::PackageError)：变体保留数据字段，用户可见的整句在构造点按请求的语言
//! 用目录拼好，`Display` 只透传 `message`。

use crate::l10n::text;
use crate::plugin::interpolate::InterpolateError;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::ManifestError;
use crate::plugin::package_files::{FileError, MAX_PACKAGE_BYTES};

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("{0}")]
    Yaml(String),
    #[error("{0}")]
    NotAMapping(String),
    #[error("{0}")]
    MissingManifest(String),
    #[error("{0}")]
    Manifest(#[from] ManifestError),
    #[error("{message}")]
    TopLevel {
        key: String,
        reason: String,
        message: String,
    },
    #[error("{message}")]
    Service { service: String, message: String },
    #[error("{message}")]
    Interpolate {
        service: String,
        message: String,
        #[source]
        source: Box<InterpolateError>,
    },
    #[error("{message}")]
    UnknownService {
        field: &'static str,
        service: String,
        message: String,
    },
    #[error("{0}")]
    Volume(String),
    #[error("{message}")]
    Config { name: String, message: String },
    #[error("{message}")]
    ConfigInterpolate {
        name: String,
        message: String,
        #[source]
        source: Box<InterpolateError>,
    },
    #[error("{message}")]
    Reference { field: String, message: String },
    #[error("{message}")]
    File {
        field: String,
        path: String,
        message: String,
        #[source]
        source: Box<FileError>,
    },
    #[error("{0}")]
    ComposeFile(String),
    #[error("{0}")]
    Archive(String),
    #[error("{0}")]
    TooBig(String),
    /// 静态网页的文件目录不合规则（ADR-0080 第 3 节）。
    #[error("{0}")]
    StaticDir(String),
    #[error("{0}")]
    GpuImages(String),
}

pub(super) fn static_error(
    langs: &Languages,
    root: &str,
    problem: &super::static_files::Problem,
) -> PackageError {
    PackageError::StaticDir(problem.message(root, langs))
}

pub(super) fn yaml_error(langs: &Languages, error: String) -> PackageError {
    PackageError::Yaml(text(
        langs,
        "plugin-install-pkg-yaml",
        &[("error", error.into())],
    ))
}

pub(super) fn top_level_error(langs: &Languages, key: &str, reason: String) -> PackageError {
    let message = text(
        langs,
        "plugin-install-pkg-top-level",
        &[("key", key.into()), ("reason", reason.as_str().into())],
    );
    PackageError::TopLevel {
        key: key.to_owned(),
        reason,
        message,
    }
}

/// 服务级别的错误：内层消息按 [`pkg-service`](crate::l10n) 套上「服务 `x`：」。
pub(super) fn service_error(langs: &Languages, service: &str, message: String) -> PackageError {
    PackageError::Service {
        service: service.to_owned(),
        message: text(
            langs,
            "plugin-install-pkg-service",
            &[
                ("service", service.into()),
                ("error", message.as_str().into()),
            ],
        ),
    }
}

pub(super) fn interpolate_error(
    langs: &Languages,
    service: String,
    source: InterpolateError,
) -> PackageError {
    let message = text(
        langs,
        "plugin-install-pkg-interpolate",
        &[
            ("service", service.as_str().into()),
            ("error", source.to_string().into()),
        ],
    );
    PackageError::Interpolate {
        service,
        message,
        source: Box::new(source),
    }
}

pub(super) fn unknown_service_error(
    langs: &Languages,
    field: &'static str,
    service: String,
) -> PackageError {
    let message = text(
        langs,
        "plugin-install-pkg-unknown-service",
        &[
            ("field", field.into()),
            ("service", service.as_str().into()),
        ],
    );
    PackageError::UnknownService {
        field,
        service,
        message,
    }
}

pub(super) fn config_error(langs: &Languages, name: &str, message: String) -> PackageError {
    PackageError::Config {
        name: name.to_owned(),
        message: text(
            langs,
            "plugin-install-pkg-config",
            &[("name", name.into()), ("error", message.as_str().into())],
        ),
    }
}

pub(super) fn config_interpolate_error(
    langs: &Languages,
    name: String,
    source: InterpolateError,
) -> PackageError {
    let message = text(
        langs,
        "plugin-install-pkg-config-interpolate",
        &[
            ("name", name.as_str().into()),
            ("error", source.to_string().into()),
        ],
    );
    PackageError::ConfigInterpolate {
        name,
        message,
        source: Box::new(source),
    }
}

pub(super) fn reference_error(langs: &Languages, field: &str, message: String) -> PackageError {
    PackageError::Reference {
        field: field.to_owned(),
        message: text(
            langs,
            "plugin-install-pkg-reference",
            &[("field", field.into()), ("error", message.as_str().into())],
        ),
    }
}

pub(super) fn file_error(
    langs: &Languages,
    field: &str,
    path: &str,
    source: FileError,
) -> PackageError {
    let message = text(
        langs,
        "plugin-install-pkg-file",
        &[
            ("field", field.into()),
            ("path", path.into()),
            ("error", source.to_string().into()),
        ],
    );
    PackageError::File {
        field: field.to_owned(),
        path: path.to_owned(),
        message,
        source: Box::new(source),
    }
}

pub(super) fn compose_file_error(langs: &Languages, source: FileError) -> PackageError {
    PackageError::ComposeFile(text(
        langs,
        "plugin-install-pkg-compose-file",
        &[("error", source.to_string().into())],
    ))
}

pub(super) fn archive_error(langs: &Languages, message: String) -> PackageError {
    PackageError::Archive(text(
        langs,
        "plugin-install-pkg-archive",
        &[("error", message.into())],
    ))
}

pub(super) fn too_big_error(langs: &Languages, total: u64) -> PackageError {
    PackageError::TooBig(text(
        langs,
        "plugin-install-pkg-too-big",
        &[
            ("kib", total.div_ceil(1024).into()),
            ("max", (MAX_PACKAGE_BYTES >> 10).into()),
        ],
    ))
}
