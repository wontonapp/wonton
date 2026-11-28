//! 插件包打成的 tar 归档和它的摘要。

use super::PluginPackage;

impl PluginPackage {
    /// 插件包打成的 tar 归档，[`Self::from_archive`] 读得回来。
    pub fn archive(&self) -> Vec<u8> {
        let mut files = self.files.clone();
        files.extend(
            self.static_blobs()
                .map(|(path, data)| (path, data.to_vec())),
        );
        super::super::package_files::write_archive(self.source.as_bytes(), &files)
    }

    /// 插件包摘要，见 [`package_files::digest`]：静态网页的文件也算在内，按包里的路径。
    pub fn digest(&self) -> String {
        let root = self.static_root().unwrap_or_default();
        let hashes = self
            .static_hashes
            .iter()
            .map(|(path, hash)| (format!("{root}/{path}"), hash.clone()));
        super::super::package_files::digest_with(self.source.as_bytes(), &self.files, hashes)
    }
}
