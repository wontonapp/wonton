//! 插件的安装、升级、来源与开关，拆出 [client mod] 保持文件在门禁内。

use super::*;

#[uniffi::export]
impl ClientCore {
    /// 安装前的预览：家里的服务器解析来源、取插件包，给出要主人确认的权限和说明，确认页渲染它。`source` 是
    /// [`StoreItem::source`]，或者主人输入的 `github:<owner>/<repo>[@<tag>]`。
    pub async fn preview_install(&self, source: String) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_install(source).await })
            .await
    }
    /// 升级前的预览：升到已安装列表里提示的版本。
    pub async fn preview_upgrade(&self, id: String) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upgrade(&id).await })
            .await
    }
    /// 主人选的压缩包的预览（ADR-0031）：`archive` 是文件的内容，壳只负责选文件和读字节。确认后按预览安装，不用再传。
    pub async fn preview_upload(&self, archive: Vec<u8>) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upload(archive).await })
            .await
    }
    /// 换版本的预览：换到主人选的来源，可以比装着的旧，确认页上会醒目提示。
    pub async fn preview_upgrade_from(
        &self,
        id: String,
        source: String,
    ) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upgrade_from(&id, source).await })
            .await
    }
    /// 换版本的预览：换到主人选的压缩包。
    pub async fn preview_upgrade_upload(
        &self,
        id: String,
        archive: Vec<u8>,
    ) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upgrade_upload(&id, archive).await })
            .await
    }
    /// 添加来源：地址可以是仓库地址、`owner/repo`，或者索引的 https 地址。
    pub async fn add_store_source(&self, address: String) -> Result<AddedSource, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.add_store_source(address).await })
            .await
    }
    /// 删除添加的来源，已安装的插件不动。
    pub async fn remove_store_source(&self, address: String) -> Result<(), CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.remove_store_source(&address).await })
            .await
    }
    /// 开关插件网页的局域网端口（ADR-0023）。核心要重建容器，做完才回应；返回开关之后的详情。
    pub async fn set_lan_webui(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.set_lan_webui(&id, enabled).await })
            .await
    }
    /// 开关插件的一项可选权限（ADR-0057）：打开时核心重建拿到字段的服务，做完才回应；返回开关之后的详情。
    pub async fn set_plugin_permission(
        &self,
        id: String,
        permission: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move {
                plugins
                    .set_plugin_permission(&id, &permission, enabled)
                    .await
            })
            .await
    }
    /// 填一项插件密钥或清空可选项（ADR-0064）：核心重建用到它的服务，做完才回应；返回之后的详情。
    pub async fn set_plugin_secret(
        &self,
        id: String,
        name: String,
        value: Option<SecretString>,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.set_secret_now(id, name, value).await })
            .await
    }
    /// 开关“每次都问”（ADR-0033）：打开后这个 Agent 每次更新都发卡片；返回开关之后的详情。
    pub async fn set_ask_every_update(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.set_ask_every_update(&id, enabled).await })
            .await
    }
    /// 撤销 Bot 往这个路径存文件的授权（ADR-0027），只拦后面的调用；返回撤销之后的详情。
    pub async fn revoke_save_grant(
        &self,
        id: String,
        folder: String,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.revoke_save_grant(&id, &folder).await })
            .await
    }
    /// 取插件的图标或截图：PNG 或 JPEG 的字节，经隧道取回并缓存，同 [`load_media`](Self::load_media)。
    pub async fn load_plugin_image(&self, image: PluginImage) -> Result<Vec<u8>, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.load_image(&image).await })
            .await
    }
    /// 在最近取到的商店列表里按关键词找插件：名字、说明、标签和 ID，不分大小写，官方的在前。还没取过商店时为空。
    pub fn search_store(&self, query: String) -> Vec<StoreItem> {
        self.scope().plugins.search_store(&query)
    }
}
