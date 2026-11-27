//! 账号与资料：这台设备上的人、头像、关于页和 App 锁，拆出 [client mod] 保持文件在门禁内。

use super::*;

#[uniffi::export]
impl ClientCore {
    /// 取家里服务器上这台设备的人（ADR-0045），设置页和填资料的页面渲染它。
    pub async fn me(&self) -> Result<Me, CoreError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(Me {
                user_id: "@owner:home.example".into(),
                username: "owner".into(),
                is_owner: true,
                role_id: None,
                role_name: None,
                permissions: None,
                display_name: Some("Owner".into()),
                email: None,
                avatar: Some(AvatarImage {
                    media: MediaHandle {
                        token: "m-owner-avatar".into(),
                    },
                    kind: AvatarKind::Still,
                    width: None,
                    height: None,
                    poster: None,
                }),
            });
        }
        let scope = self.scope();
        scope
            .run({
                let scope = scope.clone();
                async move {
                    let mut me = scope.plugins.me().await?;
                    my_avatar::attach(&mut me, &scope).await;
                    Ok(me)
                }
            })
            .await
    }
    /// 保存资料（主人写 owner.json，家人写自己那条），名字同步成 Matrix 上的显示名。不合规则时返回
    /// `InvalidInput`，原因由家里的服务器写好。保存后要重新取安装页的预览。返回的资料带服务器上的头像。
    pub async fn save_me(&self, update: OwnerProfileUpdate) -> Result<Me, CoreError> {
        let scope = self.scope();
        scope
            .run({
                let scope = scope.clone();
                async move {
                    let mut saved = scope.plugins.save_profile(update).await?;
                    if let Some(name) = saved.display_name.clone() {
                        scope.chat.set_owner_display_name(name);
                    }
                    my_avatar::attach(&mut saved, &scope).await;
                    Ok(saved)
                }
            })
            .await
    }
    /// 换自己的头像：文件由壳按 [`max_avatar_edge`](Self::max_avatar_edge)、[`max_avatar_bytes`](Self::max_avatar_bytes)
    /// 裁好、压好，核心按文件头检查后传到 Matrix 的媒体库，再把 `mxc://` 写进自己的资料 `avatar_url`。资料只放静态图。
    /// 成功返回新的 [`Me`]，壳不用再取一次；家人和主人一样能用。
    pub async fn set_my_avatar(&self, upload: AvatarUpload) -> Result<Me, MyAvatarProblem> {
        let scope = self.scope();
        scope
            .run_avatar(my_avatar::set(scope.clone(), upload))
            .await
    }
    /// 清掉自己的头像，壳显示名字首字母。本来就没有头像时也算成功：清两次的结果一样。
    pub async fn clear_my_avatar(&self) -> Result<Me, MyAvatarProblem> {
        let scope = self.scope();
        scope.run_avatar(my_avatar::clear(scope.clone())).await
    }
    /// 发起私聊时能直接选的人：核心给的可聊 Bot 在前，然后是私聊过的人。`query` 不为空时按名字和 ID 筛。
    pub fn chat_contacts(&self, query: String) -> Vec<ChatContact> {
        self.scope().chat.contacts(&query)
    }
    /// 把发起私聊时输入的用户名或 Matrix ID 变成完整的 ID：只输用户名时补上本账号的服务器。不合法时返回
    /// `InvalidInput`，还没登录时返回 `NotReady`。
    pub fn resolve_user_id(&self, input: String) -> Result<String, CoreError> {
        self.scope().chat.resolve_user_id(&input)
    }
    /// 关于页里的隐私说明，按当前语言返回。
    pub fn privacy(&self) -> PrivacyViewModel {
        crate::about::privacy()
    }
    /// 关于页里的开源许可：`licenses_json` 是 App 资源里构建时生成的清单，解析不了时返回 `InvalidInput`。
    pub fn licenses(&self, licenses_json: String) -> Result<LicensesViewModel, CoreError> {
        crate::about::licenses(&licenses_json)
    }
    /// 当前的 App 锁状态，用于首帧渲染。
    pub fn app_lock(&self) -> AppLockViewModel {
        self.app_lock.view()
    }
    /// 订阅 App 锁，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_app_lock(&self) -> Arc<AppLockSubscription> {
        Arc::new(AppLockSubscription::new(self.app_lock.subscribe()))
    }
    /// 开关 App 锁、选退到后台后多久要求解锁，存在本机，写不了时返回 `Storage`。开着时下一次进后台才锁，
    /// 主人正看着的界面不盖上；设备没设锁屏也能开，验证不了时停在锁定页。
    pub fn set_app_lock(&self, enabled: bool, timeout: AppLockTimeout) -> Result<(), CoreError> {
        self.app_lock.set(enabled, timeout)
    }
    /// 壳的系统验证通过后调用，幂等。
    pub fn unlock_app(&self) {
        self.app_lock.unlock();
    }
}
