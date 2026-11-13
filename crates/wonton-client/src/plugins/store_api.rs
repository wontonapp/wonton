//! 商店查询、插件详情和各设置项的 API：只包 [`CoreApi`] 的调用，成败转 [`CoreError`]。

use super::*;

impl PluginsLink {
    pub(crate) async fn store_plugin(&self, source: &str) -> Result<PluginDetail, CoreError> {
        let route = PluginDetailRoute::Store {
            source: source.to_owned(),
        };
        self.fetch_detail(&route).await.map_err(to_core_error)
    }

    pub(crate) async fn installed_plugin(&self, id: &str) -> Result<PluginDetail, CoreError> {
        let route = PluginDetailRoute::Installed { id: id.to_owned() };
        self.fetch_detail(&route).await.map_err(to_core_error)
    }

    pub(crate) async fn preview_install(
        &self,
        source: String,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post(
            &["plugins", "preview"],
            &InstallSource { source },
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    pub(crate) async fn preview_upgrade(&self, id: &str) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post(
            &["plugins", id, "upgrade", "preview"],
            &serde_json::json!({}),
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    /// 主人选的压缩包的预览：家里的服务器校验并暂存，预览的 `source` 就是它的来源。
    pub(crate) async fn preview_upload(
        &self,
        archive: Vec<u8>,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post_archive(&["plugins", "preview"], archive, SOURCE_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 换版本的预览：换到主人选的来源（ADR-0031）。
    pub(crate) async fn preview_upgrade_from(
        &self,
        id: &str,
        source: String,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post(
            &["plugins", id, "upgrade", "preview"],
            &InstallSource { source },
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    /// 换版本的预览：换到主人选的压缩包（ADR-0031）。
    pub(crate) async fn preview_upgrade_upload(
        &self,
        id: &str,
        archive: Vec<u8>,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post_archive(
            &["plugins", id, "upgrade", "preview"],
            archive,
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    /// 添加来源：地址原样交给家里的服务器，它判断是插件还是索引。添加之后商店里多一段。
    pub(crate) async fn add_store_source(
        self: &Arc<Self>,
        address: String,
    ) -> Result<AddedSource, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let added = api
            .post(
                &["store", "sources"],
                &AddSourceRequest { address },
                SOURCE_TIMEOUT,
            )
            .await
            .map_err(to_core_error)?;
        self.refresh_store(false);
        Ok(added)
    }

    /// 删除来源，只影响发现，已安装的插件不动。`address` 是 [`AddedSource::address`]。
    pub(crate) async fn remove_store_source(
        self: &Arc<Self>,
        address: &str,
    ) -> Result<(), CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.delete(&["store", "sources"], &[("address", address)], READ_TIMEOUT)
            .await
            .map_err(to_core_error)?;
        self.refresh_store(false);
        Ok(())
    }

    pub(crate) async fn set_lan_webui(
        self: &Arc<Self>,
        id: &str,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = LanWebuiUpdate { enabled };
        let result = api
            .put(&["plugins", id, "lan-webui"], &update, INSTALL_TIMEOUT)
            .await
            .map_err(to_core_error);
        self.refresh_installed();
        result
    }

    /// 开关一项可选权限（ADR-0057）。核心要重建容器，做完才回应；返回开关之后的详情。
    pub(crate) async fn set_plugin_permission(
        self: &Arc<Self>,
        id: &str,
        permission: &str,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = PermissionUpdate { enabled };
        let result = api
            .put(
                &["plugins", id, "permissions", permission],
                &update,
                INSTALL_TIMEOUT,
            )
            .await
            .map_err(to_core_error);
        self.refresh_installed();
        result
    }

    /// 开关“每次都问”（ADR-0033）：打开后这个 Agent 每次更新都发卡片；返回开关之后的详情。
    pub(crate) async fn set_ask_every_update(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = AskEveryUpdate { enabled };
        api.put(&["plugins", id, "ask-every-update"], &update, READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) async fn revoke_save_grant(
        &self,
        id: &str,
        folder: &str,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.delete(
            &["plugins", id, "save-grants"],
            &[("folder", folder)],
            READ_TIMEOUT,
        )
        .await
        .map_err(to_core_error)?;
        self.installed_plugin(id).await
    }

    /// 图片的地址内容不变（ADR-0017），取到一次就存在本机。缓存写不进去不影响这次的结果，下次再取。
    pub(crate) async fn load_image(&self, image: &PluginImage) -> Result<Vec<u8>, CoreError> {
        let cached = self.images_dir.join(cache_name(&image.path));
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(bytes);
        }
        let api = self.api().map_err(to_core_error)?;
        let bytes = api.image(image).await.map_err(to_core_error)?;
        if let Err(error) = save_atomically(&self.images_dir, &cached, &bytes).await {
            tracing::warn!("can't write the app image to the cache: {error}");
        }
        Ok(bytes)
    }

    /// 在取到的商店列表里找：名字、说明、标签和 ID，不分大小写，官方的在前。
    pub(crate) fn search_store(&self, query: &str) -> Vec<StoreItem> {
        let view = self.view.borrow();
        let Some(store) = view.store.as_ref() else {
            return Vec::new();
        };
        let query = query.trim().to_lowercase();
        store
            .official
            .plugins
            .iter()
            .chain(
                store
                    .added
                    .iter()
                    .flat_map(|source| &source.section.plugins),
            )
            .chain(&store.community.plugins)
            .filter(|item| {
                item.title.to_lowercase().contains(&query)
                    || item.summary.to_lowercase().contains(&query)
                    || item.id.to_lowercase().contains(&query)
                    || item
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }
}
