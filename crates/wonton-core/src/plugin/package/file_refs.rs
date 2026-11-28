//! 插件包引用的文件：清单里点名的图标、表单、头像、通知和 `configs` 的文件，收集、核对，随记录保存。

use std::collections::{BTreeMap, BTreeSet};

use super::super::images::IconFormat;
use super::super::languages::Languages;
use super::super::package_files;
use super::errors::reference_error;
use super::{ConfigSource, PackageError, PackageFiles, PluginPackage};
use crate::l10n::text;

impl PluginPackage {
    /// 包里的引用路径，含可选的状态头像首帧；缺少首帧可以省略，其他引用都必须存在。
    pub fn referenced_files(source: &str, langs: &Languages) -> Result<Vec<String>, PackageError> {
        let package = Self::structure(source, langs)?;
        let refs = package.references(langs)?;
        Ok(refs
            .into_iter()
            .chain(package.optional_references(langs)?)
            .map(|(_, path)| path)
            .collect())
    }

    pub fn optional_files(source: &str, langs: &Languages) -> Result<Vec<String>, PackageError> {
        Ok(Self::structure(source, langs)?
            .optional_references(langs)?
            .into_iter()
            .map(|(_, path)| path)
            .collect())
    }

    /// 被引用的文件：（引用它的字段，规范化后的路径）。
    pub(super) fn references(
        &self,
        langs: &Languages,
    ) -> Result<Vec<(String, String)>, PackageError> {
        let manifest = &self.manifest;
        let mut refs = Vec::new();
        let mut add = |field: &str, raw: &str| {
            let path = package_files::normalize(raw)
                .map_err(|message| reference_error(langs, field, message))?;
            refs.push((field.to_owned(), path));
            Ok::<_, PackageError>(())
        };
        let mut image_refs = Vec::new();
        if let Some(icon) = &manifest.icon {
            image_refs.push(("icon".to_owned(), icon.as_str()));
        }
        for (index, tab) in manifest
            .tabs
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            image_refs.extend(tab.icon_refs(index));
        }
        for (field, icon) in image_refs {
            // 图标随提交固定，App 也不向第三方取图（ADR-0017），所以不收网址。
            if IconFormat::from_path(icon).is_none() || icon.contains("://") {
                return Err(reference_error(
                    langs,
                    &field,
                    text(
                        langs,
                        "plugin-install-pkg-icon-file",
                        &[("icon", icon.into())],
                    ),
                ));
            }
            add(&field, icon)?;
        }
        if let Some(settings) = &manifest.settings {
            add("settings", settings)?;
        }
        if let Some(avatar) = manifest.bot.as_ref().and_then(|b| b.avatar.as_ref()) {
            add("bot.avatar", avatar)?;
        }
        if let Some(avatars) = manifest.bot.as_ref().and_then(|b| b.avatars.as_ref()) {
            for (state, path) in avatars {
                add(&format!("bot.avatars.{state}"), path)?;
            }
        }
        for notice in &manifest.notices {
            add("notices", notice)?;
        }
        for (name, config) in &self.configs {
            if let ConfigSource::File(path) = config {
                refs.push((format!("configs.{name}.file"), path.clone()));
            }
        }
        Ok(refs)
    }

    pub(super) fn optional_references(
        &self,
        langs: &Languages,
    ) -> Result<Vec<(String, String)>, PackageError> {
        let required: BTreeSet<_> = self
            .references(langs)?
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        let mut optional = BTreeMap::new();
        if let Some(avatars) = self
            .manifest
            .bot
            .as_ref()
            .and_then(|bot| bot.avatars.as_ref())
        {
            for (state, raw) in avatars {
                let path = package_files::normalize(raw).map_err(|message| {
                    reference_error(langs, &format!("bot.avatars.{state}"), message)
                })?;
                let poster = super::super::bot_avatars::poster_path(&path);
                if poster != path && !required.contains(&poster) {
                    optional.insert(poster, format!("bot.avatars.{state}.poster"));
                }
            }
        }
        Ok(optional
            .into_iter()
            .map(|(path, field)| (field, path))
            .collect())
    }

    /// 被引用的文件，安装时随记录保存。
    pub fn files(&self) -> &PackageFiles {
        &self.files
    }

    /// 清单 `x-wonton.settings` 指向的设置 Schema 原文；没声明时为空（引用在解析包时已核对存在并是 UTF-8）。
    pub fn settings(&self) -> Option<&[u8]> {
        let path = self.manifest.settings.as_deref()?;
        self.files.get(path).map(Vec::as_slice)
    }
}
