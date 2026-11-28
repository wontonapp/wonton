//! 插件包的静态网页部分：`runtime: static_web` 时的根目录、带着的文件和它们的 sha256。

use super::super::languages::Languages;
use super::errors::static_error;
use super::{PackageError, PluginPackage, static_files};

impl PluginPackage {
    /// `runtime: static_web` 时包里静态文件的目录（`static_web.root`），其他形态为空。
    pub fn static_root(&self) -> Option<String> {
        (self.manifest.runtime == super::super::manifest::Runtime::StaticWeb)
            .then(|| self.manifest.static_web.clone().unwrap_or_default().root)
    }

    /// 配上静态文件（相对 `static_web.root`），按目录规则核对（ADR-0080 第 3 节）。
    pub fn with_static(
        mut self,
        files: static_files::StaticFiles,
        langs: &Languages,
    ) -> Result<Self, PackageError> {
        let root = self.static_root().unwrap_or_default();
        let spec = self.manifest.static_web.clone().unwrap_or_default();
        static_files::check(&files, &spec).map_err(|p| static_error(langs, &root, &p))?;
        self.static_hashes = static_files::hashes(&files);
        self.static_files = Some(files);
        Ok(self)
    }

    /// 从安装记录恢复的静态网页：只有每个文件的 sha256，文件在托管目录里。
    pub fn with_static_hashes(mut self, hashes: static_files::StaticHashes) -> Self {
        self.static_hashes = hashes;
        self
    }

    /// 带着的静态文件；从安装记录恢复的、不是静态网页的为空。
    pub fn static_files(&self) -> Option<&static_files::StaticFiles> {
        self.static_files.as_ref()
    }

    pub fn static_hashes(&self) -> &static_files::StaticHashes {
        &self.static_hashes
    }

    /// 静态文件按包里的路径（`<static_web.root>/<相对路径>`）列出内容。
    pub fn static_blobs(&self) -> impl Iterator<Item = (String, &[u8])> {
        let root = self.static_root().unwrap_or_default();
        self.static_files
            .iter()
            .flatten()
            .map(move |(path, data)| (format!("{root}/{path}"), data.as_slice()))
    }
}
