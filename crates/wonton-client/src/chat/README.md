# 聊天客户端核心

Bot 资料与状态的协议见 [ADR-0044](../../../../docs/adr/0044-bot-profile-and-status-avatars.md)。

`ChatViewModel::bots` 的成员以已安装插件列表为准，不从房间列表推断。资料更新走 Matrix，管理请求走已有的 `CoreApi`；客户端不持有 Bot 的登录凭据。成员资料事件覆盖未打开时间线的房间，打开聊天也会补取自定义资料字段。

状态长轮询只在前台且连接为 `Connected` 时运行。失败即清空状态，恢复后先取完整快照，再继续携带 revision；防抖的时钟由调用方传入，定时器不依赖下一次服务器响应。typing 只认该 Bot 的已加入私聊，各房间分别记录、合并为 Bot 的一个状态。

`ClientCore::avatar_file` 返回播放器可直接打开的缓存文件，不是 URL。iOS 使用 `Application Support/<应用名>` 对应的 `Caches/<应用名>/avatars`，Android 使用 `filesDir` 对应的 `cache/avatars`，其他数据目录使用其下的 `cache/avatars`。文件名来自 MXC 哈希，身份文件同时保存完整 MXC；命中时核对身份，避免哈希碰撞返回错误媒体。媒体和身份一起原子发布，同一 MXC 的并发请求及被取消的等待者共用下载；登录尚未完成时仍可读取已缓存的文件。

打开着的房间、待处理卡片所在的房间和正在加入的房间共同决定 Sliding Sync 订阅。列表行短暂错标成受邀时，仍以 `m.direct` 和本地使用状态保留订阅；同步会话过期后，在同一把锁下先恢复名单、再启动同步，避免空订阅请求先行或健康请求被补挂取消。
