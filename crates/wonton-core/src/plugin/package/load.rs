//! 插件包的装载入口：从 compose 文本、被引用的文件、压缩包或目录读出 [`PluginPackage`]。

use std::os::fd::AsFd;
use std::path::Path;

use super::super::languages::Languages;
use super::super::package_archive;
use super::super::package_files::{
    self, COMPOSE_FILE, FileError, MAX_FILE_BYTES, MAX_PACKAGE_BYTES, PackageFiles,
};
use super::errors::{
    archive_error, compose_file_error, file_error, static_error, too_big_error, yaml_error,
};
use super::{PackageError, PluginPackage, static_files};
use crate::l10n::text;

impl PluginPackage {
    /// 配上被引用的文件：每个都要有，且不超过大小上限。
    fn attach(mut self, mut files: PackageFiles, langs: &Languages) -> Result<Self, PackageError> {
        let optional = self.optional_references(langs)?;
        let required = self.references(langs)?;
        for (field, path) in required.iter().chain(&optional) {
            if self.files.contains_key(path) {
                continue;
            }
            let Some(data) = files.remove(path) else {
                if optional.iter().any(|(_, p)| p == path) {
                    continue;
                }
                return Err(file_error(langs, field, path, FileError::Missing));
            };
            if data.len() as u64 > MAX_FILE_BYTES {
                return Err(file_error(langs, field, path, FileError::TooBig));
            }
            if field == "notices" && std::str::from_utf8(&data).is_err() {
                return Err(file_error(langs, field, path, FileError::NotText));
            }
            self.files.insert(path.clone(), data);
        }
        let total = self.source.len() + self.files.values().map(Vec::len).sum::<usize>();
        if total as u64 > MAX_PACKAGE_BYTES {
            return Err(too_big_error(langs, total as u64));
        }
        Ok(self)
    }

    /// 只有 compose 文本的插件包：引用了包里的文件时失败。
    pub fn parse(source: &str, langs: &Languages) -> Result<Self, PackageError> {
        Self::with_files(source, PackageFiles::new(), langs)
    }

    /// compose 文本加上被引用的文件（如安装记录里保存的那些）。`files` 里没被引用的文件不保留。
    pub fn with_files(
        source: &str,
        files: PackageFiles,
        langs: &Languages,
    ) -> Result<Self, PackageError> {
        Self::structure(source, langs)?.attach(files, langs)
    }

    /// 插件包打成的压缩包，规则见 [`package_archive`]：只解压 `docker-compose.yml` 和它引用的文件。
    pub fn from_archive(body: &[u8], langs: &Languages) -> Result<Self, PackageError> {
        let archive = package_archive::Archive::open(body).map_err(|e| archive_error(langs, e))?;
        let root = archive.root(langs).map_err(|e| archive_error(langs, e))?;
        let compose = archive
            .read(&root, &[COMPOSE_FILE.to_owned()], langs)
            .map_err(|e| archive_error(langs, e))?
            .remove(COMPOSE_FILE)
            .ok_or_else(|| compose_file_error(langs, FileError::Missing))?;
        let source = String::from_utf8(compose)
            .map_err(|_| yaml_error(langs, text(langs, "plugin-install-pkg-not-utf8", &[])))?;
        let files = archive
            .read(&root, &Self::referenced_files(&source, langs)?, langs)
            .map_err(|e| archive_error(langs, e))?;
        let package = Self::with_files(&source, files, langs)?;
        let Some(static_root) = package.static_root() else {
            return Ok(package);
        };
        let prefix = format!("{root}{static_root}/");
        let files = archive
            .read_tree(&prefix, static_files::MAX_FILE_BYTES, langs)
            .map_err(|e| archive_error(langs, e))?;
        package.with_static(files, langs)
    }

    /// 读插件包目录：`docker-compose.yml` 和它引用的文件，路径和软链接的规则见 [`super::package_files`]。
    pub fn load_dir(dir: &Path, langs: &Languages) -> Result<Self, PackageError> {
        let opened =
            super::super::package_files::open_dir(dir).map_err(|e| compose_file_error(langs, e))?;
        Self::load_dir_at(opened.as_fd(), langs)
    }

    /// 同 [`Self::load_dir`]，从已打开的目录读：调用方（如 outbox 的解析）钉住目录，中途换不掉。
    pub fn load_dir_at(
        dir: std::os::fd::BorrowedFd<'_>,
        langs: &Languages,
    ) -> Result<Self, PackageError> {
        let source =
            package_files::read_at(dir, COMPOSE_FILE).map_err(|e| compose_file_error(langs, e))?;
        let source = String::from_utf8(source)
            .map_err(|_| yaml_error(langs, text(langs, "plugin-install-pkg-not-utf8", &[])))?;
        let package = Self::structure(&source, langs)?;
        let mut files = PackageFiles::new();
        let optional = package.optional_references(langs)?;
        let required = package.references(langs)?;
        for (field, path) in required.iter().chain(&optional) {
            let data = match package_files::read_at(dir, path) {
                Ok(data) => data,
                Err(FileError::Missing) if optional.iter().any(|(_, p)| p == path) => continue,
                Err(source) => return Err(file_error(langs, field, path, source)),
            };
            files.insert(path.clone(), data);
        }
        let package = package.attach(files, langs)?;
        match package.static_root() {
            Some(root) => {
                let files = static_files::read_dir(dir, &root)
                    .map_err(|problem| static_error(langs, &root, &problem))?;
                package.with_static(files, langs)
            }
            None => Ok(package),
        }
    }
}
