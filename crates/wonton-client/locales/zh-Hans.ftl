## 默认收起的技术详情。原因、错误码和上游原文保持不变。
## 不限长度。参数是诊断数据，不是翻译键。

# CoreError 前缀。$reason 是原始错误详情。
core-error-storage = 本机存储读写失败：{ $reason }
core-error-not-ready = 尚未就绪：{ $reason }
core-error-transport = 连不上家里的服务器：{ $reason }
core-error-unauthorized = 家里的服务器拒绝了请求：{ $reason }
core-error-upstream = 家里的服务器出错：{ $reason }
core-error-server-too-old = 家里的服务器太旧，还做不了这件事。升级服务器后再试。

# 添加设备时，还不能请求配对码的原因。
invite-not-paired = 还没有配对家里的服务器。
# $reason 是连接信息解析器的原始错误。
invite-invalid-ticket = 保存的连接信息已损坏：{ $reason }
invite-not-connected = 还没连上家里的服务器。
# $reason 是代理地址解析器的原始错误。
invite-invalid-proxy = 本地代理地址不合法：{ $reason }
# $reason 是 HTTP 客户端的原始错误。
invite-http-client = 无法创建 HTTP 客户端：{ $reason }
# $seconds 是请求超时的秒数。
invite-timeout = { $seconds } 秒内没有回应
# $reason 是上游原文，或翻译后的超时、协议错误详情。
invite-request-failed = 向家里的服务器要配对码失败：{ $reason }
# $reason 是 JSON 解析器的原始错误。
invite-invalid-response = 家里的服务器给的配对码格式不对：{ $reason }
# $status 是原始 HTTP 状态码。
invite-server-status = 家里的服务器回了 HTTP { $status }
# $message 和 $kind 是服务器的原始错误说明和错误码。
invite-server-error = { $message }（{ $kind }）

# 其他服务器（ADR-0050）。默认收起的“详情”。
# $reason 是原始错误详情。
peers-transport = 连不上家里的服务器：{ $reason }
# $seconds 是请求超时的秒数。
peers-timeout = { $seconds } 秒内没有回应
# 还没有连接信息或本地代理，请求发不出去。
peers-not-connected = 还没连上家里的服务器。
# 壳把不是服务器配对的操作发到了这里。
peers-invalid-action = 不是服务器配对操作。

# $reason 是本地代理地址或 HTTP 客户端的原始错误。
peers-invalid-proxy = 本地代理不可用：{ $reason }
# $status 是原始 HTTP 状态码。
peers-server-status = 家里的服务器回了 HTTP { $status }
# $reason 是 JSON 解析器的原始错误。
peers-invalid-response = 家里的服务器给的回应读不懂：{ $reason }
# $reason 是隧道拒绝的原话。
peers-rejected = 家里的服务器拒绝了这台设备：{ $reason }

# 引导的步骤操作：落在错的步骤上、状态没取到时的原因。
onboarding-steps-not-in-flow = 不在引导里
onboarding-steps-already-committed = 已经点了完成
onboarding-steps-not-profile-step = 不在填资料这一步
onboarding-steps-not-agent-step = 不在选 Agent 这一步
onboarding-steps-state-missing = 还没有取到引导状态
onboarding-steps-agent-not-listed = 这个 Agent 不在可选列表里
onboarding-steps-agent-not-installable = 这个 Agent 不能安装
onboarding-steps-not-apps-step = 不在选应用这一步
onboarding-steps-need-not-listed = 这个需求不在推荐列表里
onboarding-steps-committing = 正在提交
onboarding-steps-no-installable-agent = 没有选能装的 Agent
onboarding-steps-testing = 正在测试
onboarding-steps-model-not-chosen = 还没有选模型
onboarding-steps-cannot-skip = 这一步不能跳过
onboarding-steps-cannot-go-back = 这一步不能返回
# 壳把不认识的操作分给引导时显示。
onboarding-invalid-action = 不是引导操作

# 接模型这一步。
onboarding-model-key-required = key 不能为空
onboarding-model-not-model-step = 还没到接模型这一步
onboarding-model-testing = 已经在测试
onboarding-model-providers-missing = 还没取到服务商列表
onboarding-model-provider-not-listed = 这个服务商不在列表里
onboarding-model-custom-unsupported = 模型网关不支持自定义服务商
onboarding-model-name-required = 模型名不能为空
onboarding-model-base-url-invalid = Base URL 要是 https://… 这样的地址
onboarding-model-step-or-testing = 不在接模型这一步，或者正在测试
onboarding-model-none-configured = 模型网关里还没有接好的模型
onboarding-model-state-missing = 还没有取到引导状态
onboarding-model-login-missing = 选中的 Agent 没有这种登录

# 最后一页。被拦下的项带着这条说明，直到预览刷新。
onboarding-review-permissions-changed = 要的权限和确认时的不一样了，到应用页重新装
# $installed 和 $listed 是安装来源提供的和列表里的插件 ID，不翻译。
onboarding-review-source-mismatch = 安装来源里的插件是 { $installed }，不是列表里的 { $listed }
onboarding-review-items-not-ready = 还有没准备好的项
onboarding-review-item-missing = 最后一页上没有这一项
onboarding-review-no-failure = 这一项没有失败

## chat-a 片段：crates/wonton-client/src/chat/{login,sign_in,recovery}.rs。
## 是聊天问题/错误默认收起的“详情”：CoreError 的原因和 ChatProblem 的 message。

# 登录相关。$reason 是 Matrix SDK 的原始错误。
chat-sign-in-restore-failed = 恢复 Matrix 会话失败：{ $reason }
chat-sign-in-no-connection-info = 还没有家里服务器的连接信息
chat-sign-in-login-failed = 登录 Matrix 失败：{ $reason }
chat-sign-in-no-saved-session = 登录后取不到登录状态

# 要登录令牌的请求。$reason 是上游原文，或翻译后的超时详情。
# $reason 是代理地址解析器的原始错误。
chat-login-invalid-proxy = 本地代理地址不合法：{ $reason }
# $reason 是 HTTP 客户端的原始错误。
chat-login-http-client = 建 HTTP 客户端失败：{ $reason }
# $seconds 是请求超时的秒数。
chat-login-timeout = { $seconds } 秒内没有回应
chat-login-request-failed = 向核心要 Matrix 登录令牌失败：{ $reason }
# $reason 是读 HTTP 回应的原始错误。
chat-login-response-failed = 读核心的回应失败：{ $reason }
# $reason 是 JSON 解析器的原始错误。
chat-login-invalid-token = 核心给的登录令牌格式不对：{ $reason }
# $login_type 是核心给的登录方式；$expected 是要求的登录方式。
chat-login-wrong-type = 核心给的登录方式 { $login_type } 不是 { $expected }
# $status 是原始 HTTP 状态码。
chat-login-server-status = 核心回了 { $status }
# $message 和 $kind 是服务器的原始错误说明和错误码。
chat-login-server-error = { $message }（{ $kind }）

# 恢复相关。
chat-recovery-not-connected = 还没连上家里的服务器
chat-recovery-reset-no-response = 家里的服务器没有回应，备份没有动，稍后再试
chat-recovery-empty-key = 恢复密钥不能为空
# $reason 是存储的原始错误。
chat-recovery-key-not-saved = 恢复密钥没能存到本机，请现在就抄下来：{ $reason }
# 回答或停止时段没成功时，卡片下面问题详情里显示的话。
# $message 是核心的原始错误说明，或翻译后的详情。
chat-confirmation-answer-failed = 回答没有成功：{ $message }
# $error 是 HTTP 客户端的原始错误。
chat-confirmation-answer-lost = 没等到家里服务器的回应，不确定答上了没有，可以再点一次（{ $error }）
# $message 是核心的原始错误说明，或翻译后的详情。
chat-confirmation-stop-failed = 没有停下来：{ $message }
# $error 是 HTTP 客户端的原始错误。
chat-confirmation-stop-lost = 没等到家里服务器的回应，可以再点一次（{ $error }）
# $error 是 JSON 解析器的原始错误。
chat-confirmation-response-unreadable = 核心的回应读不懂（{ $error }）
# $status 是原始 HTTP 状态码。
chat-confirmation-core-status = 核心回了 { $status }
# 开始私聊时用户名为空时显示。
chat-contacts-empty-username = 用户名不能为空。
# $reason 是 Matrix ID 解析器的原始错误。
chat-contacts-invalid-user-id = 用户 ID 不合法：{ $reason }

# 通过 Siri、快捷指令或聊天页发消息、内容为空时显示。
chat-empty-message = 消息不能为空。
# Siri 或快捷指令传进来的 Bot ID 解析不了时显示。
chat-ask-invalid-bot-id = 默认 Agent 的 ID 格式不对。
# 已有私聊的房间 ID 解析不了时显示。
chat-ask-invalid-room-id = 私聊的房间 ID 不合法。
chat-layout-invalid-bot-id = ID 的格式不对，应该像 @名字:服务器。
chat-layout-change-failed = 改显示方式没成功：{ $reason }

# Matrix 吊销本设备、账号标记为失败时显示。
chat-session-device-revoked = 这台设备在 Matrix 上已被吊销，需要主人重新配对。

# 壳把不认识的操作分给聊天时显示。
chat-invalid-action = 不是聊天操作
# 发起私聊时给的 ID 不像 @名字:服务器 时显示。
chat-invalid-user-id = ID 的格式不对，应该像 @名字:服务器
# $file_type 是选中文件原始的 MIME 类型。
chat-not-image-type = 这个文件不是图片：{ $file_type }
# 壳给了这个核心不认识的回答时显示。
chat-unknown-answer = 这个版本的 App 不认识这个回答
# 还没连上家里的服务器就回答或停止确认时显示。
chat-action-not-connected = 还没连上家里的服务器
# 还没登录 Matrix 就有聊天操作时显示。
chat-not-signed-in = 还没有登录 Matrix
# $error 是房间 ID 解析器的原始错误。
chat-invalid-room-id = 房间 ID 不合法：{ $error }
# 往只放通知的房间里发东西时显示。
chat-read-only-room = 这个房间只放通知，不能发消息
# $room 是原始的房间 ID。
chat-room-not-subscribed = 要先订阅房间 { $room } 的时间线
# $error 是事件 ID 解析器的原始错误。
chat-invalid-event-id = 消息的事件 ID 不合法：{ $error }
# 要转发或撤回的消息菜单里没有这一项（已撤回、还没发出去、没有权限等）时显示。
chat-message-action-unavailable = 这条消息不能这样操作
# 转发的目标房间不存在或还没加入时显示。
chat-forward-target-not-joined = 还没有加入要转发到的房间
# 还没选默认 Agent 就问它时显示。
chat-no-default-agent = 还没有默认 Agent：先在设置里选一个

# 后台聊天操作失败的话，放进问题提示里；$error 是 Matrix SDK 或家里的服务器的原始错误。
chat-op-ask-agent-failed = 问默认 Agent 失败：{ $error }
chat-op-display-name-failed = 同步显示名失败：{ $error }
chat-op-join-failed = 加入房间失败：{ $error }
chat-op-leave-failed = 离开房间失败：{ $error }
chat-op-resend-failed = 重发消息失败：{ $error }
chat-op-cancel-send-failed = 取消发送失败：{ $error }
chat-op-paginate-failed = 加载更早的消息失败：{ $error }
chat-op-mark-read-failed = 标为已读失败：{ $error }
chat-op-direct-chat-failed = 发起私聊失败：{ $error }
chat-op-send-text-failed = 发送消息失败：{ $error }
chat-op-send-image-failed = 发送图片失败：{ $error }
chat-op-forward-failed = 转发消息失败：{ $error }
chat-op-redact-failed = 撤回消息失败：{ $error }
## 文件传输和播放的详情。显示在传输列表条目、文件卡片和播放失败说明上。

# 传输或播放还没等开始就发现没连上时显示。
files-api-not-connected = 还没连上家里的服务器
# $reason 是代理地址解析器的原始错误。
files-api-invalid-proxy = 本地代理地址不合法：{ $reason }
# $reason 是 HTTP 客户端的原始错误。
files-api-http-client = 建 HTTP 客户端失败：{ $reason }
# $reason 是 JSON 解析器的原始错误。
files-api-invalid-response = 读不懂核心的响应：{ $reason }
# 家里的服务器不再认这台设备时显示。
files-api-unauthenticated = 家里的服务器不再认这台设备
# $code 是隧道原始错误码。
files-api-tunnel-unavailable = 隧道不可用（{ $code }）
# $status 是原始 HTTP 状态码。
files-api-status = 核心回了 { $status }

# 原文件和选择时不一样时显示。
files-upload-source-changed = 原文件改过了，要重新选择
# 核心对进度查询（HEAD）的响应缺少 Upload-Offset 头时显示。
files-upload-head-no-offset = 核心问进度的响应没有 Upload-Offset
# 壳打不开这类来源时显示。
files-upload-no-opener = 壳没有设置 FileOpener，打不开这个来源
# 来源不是普通文件、没法定位时显示。
files-upload-source-not-file = 这个来源不是能定位的普通文件，接不上断点
# $reason 是打开原文件的任务的异常信息。
files-upload-open-task-failed = 打开原文件的任务异常结束：{ $reason }
# $reason 是原始 io 错误。
files-upload-source-denied = 系统不让读原文件：{ $reason }
# $reason 是原始 io 错误。
files-upload-source-missing = 原文件没了：{ $reason }
# $reason 是原始 io 错误。
files-upload-source-read = 读不了原文件：{ $reason }
# 核心创建上传的响应缺少合法的 Location 头时显示。
files-upload-create-no-location = 核心创建上传的响应没有合法的 Location
# 核心对进度查询（HEAD）的响应缺少 Upload-Length 头时显示。
files-upload-head-no-length = 核心问进度的响应没有 Upload-Length
# 超过卡住时限没有字节在动时显示。
files-upload-stalled = 传不动了，隧道可能卡住了
# 核心一直不回答最后一个 PATCH 时显示。
files-upload-no-response = 核心一直没有回答
# 原文件比记录的短时显示。
files-upload-source-shrank = 原文件变短了

# $reason 是原始 io 错误。
files-download-write = 写不了下载的文件：{ $reason }
# 核心一直不回答下载请求时显示。
files-download-no-response = 核心一直没有回答
# 半截文件比核心那边的文件还长时显示。
files-download-shrunk = 文件变短了，从头下
# 接着下的范围和已下载的对不上时显示。
files-download-range-mismatch = 核心给的范围对不上，从头下
# 连接断了、字节没收完时显示。
files-download-incomplete = 连接断了，没收全
# 超过卡住时限收不到数据时显示。
files-download-stalled = 收不到数据，隧道可能卡住了

# 连接断开导致播放失败时显示。
files-playback-transport = 连不上家里的服务器。可以下载后用其他 App 打开。
# 这台设备放不了这个格式时显示。
files-playback-unsupported = 这台手机放不了这个格式。可以下载后用其他 App 打开。
# $detail 是家里的服务器或连接的原始原因。
files-playback-unavailable = 家里的服务器给不了这个文件：{ $detail }
# $detail 是家里的服务器或连接的原始原因。
files-playback-error = 家里的服务器报错：{ $detail }。可以下载后用其他 App 打开。
# 播放探测超时时显示。
files-playback-no-response = 核心一直没有回答

# $reason 是原始 io 错误。
files-stream-local-address = 开不了播放用的本地地址：{ $reason }
# 隧道没连上时播放失败的诊断详情。
files-stream-not-connected = 隧道没连上
# $code 是隧道原始错误码。
files-stream-tunnel-unavailable = 隧道不可用（{ $code }）
# $error 是 JSON 解析器的原始错误。
home-ticket-invalid-json = 连接信息不是合法的 JSON：{ $error }
# 没有参数。扫到的连接信息里没有能连上服务器的地址。
home-ticket-no-addresses = 连接信息里没有服务端的中继地址或直连地址
home-ticket-relays-invalid = 中继表不合法：{ $reason }
# $domain 是连接信息里的原始基础域名。
home-ticket-invalid-domain = 基础域名不合法：{ $domain }
# $error 是地址解析器的原始错误。
home-ticket-invalid-web-url = 插件网页地址不合法：{ $error }
# $domain 是连接信息的基础域名，$url 是连接信息里的原始地址。
home-ticket-web-url-under-domain = 插件网页地址要是 { $domain } 下的 http(s) 地址：{ $url }
# $error 是地址解析器的原始错误。
home-ticket-invalid-core-url = 核心地址不合法：{ $error }
# $domain 是连接信息的基础域名，$url 是连接信息里的原始地址。
home-ticket-core-url-under-domain = 核心地址要是 { $domain } 的 http(s) 地址：{ $url }
# $error 是网络初始化的原始错误。
# 没有参数。配对页的设备名一栏被清空了。
home-pairing-name-empty = 设备名是空的。
home-pairing-bind-failed = 绑定网络失败：{ $error }
# $reason 是连接信息解析器的原始错误。
home-ticket-corrupt = 保存的连接信息已损坏：{ $reason }
home-no-connection-info = 还没有家里服务器的连接信息
# $error 是本地代理原始的绑定错误。
home-proxy-start-failed = 本地代理启动失败：{ $error }
home-tunnel-closed = 隧道客户端已关闭
home-lan-unsupported = 这个版本的客户端核心不支持局域网设置
home-port-mapping-save-failed = 端口映射的开关存不下：{ $error }
# 底栏（tabs/view.rs、tabs/config.rs）和外观（appearance/mod.rs）的界面「详情」。

# 底栏 ViewModel 的错误，作为 CoreError 的 reason 或底栏的问题提示显示。
# 无参数。
tabs-view-always-pinned = 聊天和应用已经固定到底栏。
tabs-view-plugin-list-not-ready = 插件列表还没读到，暂时不能操作插件页面。
tabs-view-unknown-plugin = 没有这个插件。
tabs-view-plugin-no-page = 插件没有可打开的网页。

# 底栏设置文件（tabs.json）的错误，作为 CoreError 的 reason 显示。
# $path 是设置文件的路径，$reason 是原始的解析或读写错误。
tabs-config-read-failed = 读不了底栏设置文件 { $path }：{ $reason }
tabs-config-corrupt = 底栏设置文件 { $path } 已损坏：{ $reason }
# $count 是底栏最多能放的项数（MAX_ITEMS）。
tabs-config-too-many-items = 底栏最多只能有 { $count } 项。
tabs-config-duplicate-page = 底栏里有重复页面。
tabs-config-plugin-page-incomplete = 插件页面缺少标题和图标。
tabs-config-missing-fixed = 底栏缺少聊天。
# $path 是设置文件的路径，$reason 是原始的写入错误。
tabs-config-write-failed = 写不了底栏设置文件 { $path }：{ $reason }

# 外观设置的错误。appearance-settings-unreadable 作为外观页的问题提示（AppearanceProblem.message）显示，其余是它里面的 reason 或 CoreError::Storage 的 reason。
# $path 是设置文件的路径，$reason 是原始的解析或读写错误。
appearance-settings-unreadable = 外观设置读不了，先用默认值：{ $reason }
appearance-settings-read-failed = 读不了外观设置文件 { $path }：{ $reason }
appearance-settings-corrupt = 外观设置文件 { $path } 已损坏：{ $reason }
# $value 是设置文件里的原始颜色字符串，原样不翻。
appearance-settings-invalid-accent = 外观设置文件 { $path } 里的颜色无效：{ $value }
# $path 是设置文件的路径，$reason 是原始的写入错误。
appearance-settings-write-failed = 写不了外观设置文件 { $path }：{ $reason }

## 插件模块的技术详情，默认收起在错误、应用任务和出网代理、镜像加速设置页的“详情”里。
## 不限长度。参数是诊断数据，不是翻译键。

# 插件管理接口（crates/wonton-client/src/plugins/api.rs）。
# $reason 是原始错误详情。
plugins-api-invalid-proxy = 本地代理地址不合法：{ $reason }
plugins-api-http-client = 建 HTTP 客户端失败：{ $reason }
# $reason 是 JSON 解析器的原始错误。
plugins-api-invalid-response = 家里的服务器给的内容格式不对：{ $reason }
# $reason 是 JSON 序列化器的原始错误。
plugins-api-invalid-request = 请求体写不出来：{ $reason }
# $reason 是被拒绝的图片地址，或 URL 解析器的原始错误。
plugins-api-not-image-url = 不是插件图片的地址：{ $reason }
# $status 是原始 HTTP 状态码。
plugins-api-server-status = 家里的服务器回了 { $status }
plugins-api-not-connected = 连上家里的服务器后再试。
plugins-api-server-too-old = 家里的服务器 { $version } 太旧：这个功能要 { $minimum } 起。

# 安装、升级、卸载任务（crates/wonton-client/src/plugins/tasks.rs）。
plugins-tasks-uncertain = 连接断了，等不到家里服务器的结果，看一下已安装列表。
plugins-tasks-server-changed = 换了家里的服务器，等不到原来那台的结果。
plugins-tasks-preview-no-source = 这份预览没有来源，装不了。
# $id 是原样的任务 ID。
plugins-tasks-no-such-task = 没有这个任务：{ $id }
plugins-tasks-task-not-done = 任务还没做完。
# $title 是应用标题，按商店里的原样显示。
plugins-tasks-task-running = { $title } 正在安装、升级或卸载。

# 出网代理设置（crates/wonton-client/src/plugins/egress.rs）。
plugins-egress-save-in-progress = 上一次改动还没做完。
plugins-egress-not-loaded = 还没取到出网代理的设置。
plugins-egress-no-confirmation = 没有等待确认的改动。
plugins-egress-env-locked = 代理由 WONTON_EGRESS_PROXY 指定，不能在这里改。
plugins-egress-plugin-and-address = 使用代理插件时不能同时填写代理地址。
plugins-egress-plugin-not-installed = 还没有安装代理插件。
# $reason 是连接问题的原始详情。
plugins-egress-save-uncertain = 不确定改好了没有，连上后再看一下：{ $reason }

# 镜像加速设置（crates/wonton-client/src/plugins/registry_mirror.rs）。
plugins-registry-save-in-progress = 上一次镜像加速改动还没做完。
plugins-registry-not-loaded = 还没取到镜像加速设置。
# $reason 是连接问题的原始详情。
plugins-registry-save-uncertain = 不确定改好了没有，连上后重新读取设置：{ $reason }

# 多台服务器（crates/wonton-client/src/servers.rs、client/push_route.rs）。
servers-removed = 这台服务器已经从这台手机上移除。
servers-push-from-removed = 发来这条消息的服务器已经从这台手机上移除。
servers-push-scope-closed = 那台服务器已经关掉了。
# $seconds 是等待登记的秒数。
servers-push-register-timeout = { $seconds } 秒内没能登记推送。

chat-avatar-cache-not-file = 头像缓存路径不是文件
chat-avatar-cache-hash-collision = 头像缓存哈希冲突
chat-avatar-cache-missing-mxc = 头像缓存缺少 MXC 标识
chat-avatar-cache-path-not-utf8 = 头像缓存路径不是有效的 UTF-8
scope-task-crashed = 核心的后台任务异常结束：{ $error }
scope-switched-server = 已经切到另一台服务器

## 客户端核心剩下的部分（ADR-0043）：C1、C2 没覆盖的模块的“详情”，默认收起。同一批文件里的日志写英文，不在这里。
## 推送通知里给人看的正文和占位（未知的发送者、邀请你加入、[文件]……）还没定，不在这次范围。

# 文件传输（crates/wonton-client/src/files/mod.rs）：被拒绝的操作、失败的操作和本机文件清理的详情。
files-invalid-action = 不是文件操作
files-name-required = 文件名不能为空
files-download-already-running = 这个文件已经在下载了
files-transfer-not-found = 没有这条传输
# 这条传输不在“等着发链接”的状态时要重发链接。
files-resend-not-waiting = 这条传输没有等着重发的链接
files-transfer-server-changed = 换了家里的服务器，这个传输属于之前那一台
files-dismiss-transfer-running = 这条传输还在进行，要先取消
# 移除上传并要求删除本机文件时：原文件是系统给的，App 删不了它。
files-dismiss-upload-source-protected = 上传的原文件不归 App 管
# $reason 是原始的 io 错误。
files-delete-local-failed = 删不掉手机上的文件：{ $reason }
# 后台文件操作失败的结果。$id 是原样的文件编号，$message 是原始的失败详情。
files-delete-failed = 删除文件 { $id } 失败：{ $message }
files-send-failed = 把文件 { $id } 发到设备失败：{ $message }
files-grant-failed = 授权文件 { $id } 失败：{ $message }
files-play-failed = 播放文件 { $id } 失败：{ $message }
# $path 是传输列表的路径，$reason 是原始的读写错误。
files-store-write-failed = 写不了传输列表 { $path }：{ $reason }

# 连接的本机存储（crates/wonton-client/src/home/storage.rs）。
# $path 是文件的路径，$reason 是原始的解析或读写错误。
home-storage-device-key-corrupt = 设备密钥 { $path } 已损坏：{ $reason }
home-storage-io-failed = 读写不了 { $path }：{ $reason }

# 聊天的本机存储（crates/wonton-client/src/chat/storage.rs）。
# $path 是文件的路径，$reason 是原始的解析或读写错误。
chat-storage-session-corrupt = 聊天的登录状态 { $path } 已损坏：{ $reason }
chat-storage-io-failed = 读写不了 { $path }：{ $reason }

# 配过的服务器列表（crates/wonton-client/src/servers.rs）。
# $path 是服务器列表文件的路径，$reason 是原始的解析或读写错误。
servers-list-corrupt = 服务器列表 { $path } 已损坏：{ $reason }
servers-add-current-unpaired = 先配好这一台，再添加别的服务器
servers-name-required = 名字不能为空
# $path 是要读、写或删的路径，$reason 是原始的读写错误。
servers-storage-failed = 读写不了 { $path }：{ $reason }
# 壳把不认识的操作分给服务器列表时显示。
servers-invalid-action = 不是服务器列表操作

# 会话列表（crates/wonton-client/src/session_store.rs）。
sessions-title-required = 会话标题不能为空
# $id 是原样的会话编号。
sessions-not-found = 会话不存在：{ $id }
# 壳把不认识的操作分给会话时显示。
sessions-invalid-action = 不是会话操作

# 推送方式设置（crates/wonton-client/src/push_method.rs）。
# $path 是设置文件的路径，$reason 是原始的读写错误。
push-method-write-failed = 写不了推送方式设置文件 { $path }：{ $reason }

# 推送（crates/wonton-client/src/push/{mod,registration,notification}.rs）：被拒绝的地址、失败的登记和没能取回消息的详情。
# 通知的正文和占位（未知的发送者、邀请你加入、[文件]……）还不在这里。
# $reason 是原始错误详情。
push-matrix-pusher-failed = 在 Matrix 上注册 pusher 失败：{ $reason }
# $reason 是 URL 解析器的原始错误。
push-endpoint-invalid-url = 推送地址不是合法的 URL：{ $reason }
push-endpoint-scheme = 推送地址必须是 http 或 https 地址
push-endpoint-p256dh = p256dh 应为 65 字节的未压缩 P-256 公钥（base64url）
push-endpoint-auth = auth 应为 16 字节（base64url）
# $reason 是上游原文，或翻译后的超时详情。
push-register-failed = 登记推送失败：{ $reason }
push-unregister-failed = 取消推送登记失败：{ $reason }
# $reason 是 JSON 解析器的原始错误。
push-register-invalid-response = 核心给的推送登记结果格式不对：{ $reason }
# $reason 是本地代理或 HTTP 客户端的原始错误。
push-invalid-proxy = 本地代理地址不合法：{ $reason }
push-http-client = 建 HTTP 客户端失败：{ $reason }
# $seconds 是请求的超时秒数。
push-timeout = { $seconds } 秒内没有回应
# $status 是原始 HTTP 状态码。
push-server-status = 核心回了 { $status }
# $message 和 $kind 是服务器的原始错误文字和错误码。
push-server-error = { $message }（{ $kind }）
# $reason 是 JSON 解析器的原始错误。
push-payload-invalid = 推送内容的格式不对：{ $reason }
# $reason 是 ID 解析器的原始错误。
push-invalid-room-id = 推送里的房间 ID 不合法：{ $reason }
push-invalid-event-id = 推送里的事件 ID 不合法：{ $reason }
# $seconds 是取消息等的秒数。
push-fetch-timeout = { $seconds } 秒内没能取回这条消息
# $reason 是 Matrix SDK 的原始错误。
push-fetch-failed = 取不到这条消息：{ $reason }
push-core-closed = 客户端核心已关闭
push-not-paired = 还没有连过家里的服务器
push-home-unreachable = 连不上家里的服务器：{ $reason }
# $seconds 是登录等的秒数。
push-session-timeout = { $seconds } 秒内没能登录 Matrix
push-room-key-missing = 还没收到这条消息的房间密钥

# 启动家里的应用（crates/wonton-client/src/plugins/start.rs）。
# $id 是原样的应用编号。
plugins-start-busy = { $id } 正在启动、安装、更新或卸载

# 应用页面的桥（crates/wonton-client/src/tabs/bridge/）。
tabs-bridge-page-not-open = 这个插件页面没有打开或固定
tabs-bridge-invalid-environment = 页面栏高和安全区必须是有限的非负数

# 聊天的连接问题（crates/wonton-client/src/chat/view.rs）：CoreError::TransportUnavailable 的原因和失败操作的消息。
chat-not-connected = 连上家里的服务器后再试
chat-draft-save-failed = 存草稿失败：{ $error }
chat-draft-clear-failed = 清掉已发出的草稿失败：{ $error }
chat-draft-not-signed-in = 还没有登录 Matrix
chat-draft-read-interrupted = 草稿的读取被打断了

# 引导的本机记录（crates/wonton-client/src/onboarding/storage.rs）。
# $path 是文件的路径，$reason 是原始的读写错误。
onboarding-storage-io-failed = 读写不了 { $path }：{ $reason }

# 家里服务器的一键更新（crates/wonton-client/src/plugins/core_upgrade.rs）：不确定阶段的说明和被拒绝的操作，显示在更新页和错误的“详情”里。
# 无参数。家里的服务器给了这个 App 不认识的结果。
plugins-core-upgrade-unrecognized = 家里的服务器给了这个 App 不认识的结果，更新 App 后再看
# 代替缺的正在运行的摘要。不是一句话。
plugins-core-upgrade-unknown-digest = 未知
# 家里的服务器报告更新完成，但正在运行的和目标对不上。$running 是正在运行的摘要或“未知”，$target 是原样的目标摘要。
plugins-core-upgrade-completed-mismatch = 家里的服务器说升级完成了，但正在运行的不是目标版本：在运行 { $running }，目标 { $target }
# 过了时限没连上，结果未知。$digest 是退回旧版本要固定的原样摘要。
plugins-core-upgrade-timed-out = 过了时限还连不上家里的服务器，不知道更新的结果。连上后这里会更新
plugins-core-upgrade-timed-out-rollback = 过了时限还连不上家里的服务器，不知道更新的结果。连上后这里会更新。要退回旧版本，在主机上再执行一次安装命令，加上 WONTON_CORE_PIN={ $digest }
plugins-core-upgrade-invalid-action = 不是更新服务器的操作
plugins-core-upgrade-already-running = 已经在更新了
plugins-core-upgrade-not-cancellable = 现在不能取消
# $reason 是连接问题的原始详情。
plugins-core-upgrade-start-uncertain = 不确定发起了没有，连上后再看一下：{ $reason }

# 资料这一步：名字或邮箱为什么存不了。$max 是名字最多几个字。
onboarding-profile-name-empty = 名字不能为空
onboarding-profile-name-too-long = 名字太长：最多 { $max } 个字
onboarding-profile-name-control = 名字里不能有换行和控制字符
onboarding-profile-email-invalid = 不像邮箱：写成 me@example.com 这样，没有空白和控制字符
onboarding-profile-email-cleared = 邮箱填过以后只能改，不能清空：要邮箱的应用重建时会用到它

# 确认卡片下面放行时段结束后的那一行。$count 是打开时段那次之外放行了几次，0 表示不知道或没有。
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] 放行时段已到期。
               *[other] 放行时段已到期，放行了 { $count } 次。
            }
        [stopped]
            { $count ->
                [0] 放行时段已停止。
               *[other] 放行时段已停止，放行了 { $count } 次。
            }
        [restarted] 服务器重启过，放行时段已结束。
       *[other]
            { $count ->
                [0] 放行时段已结束。
               *[other] 放行时段已结束，放行了 { $count } 次。
            }
    }

# 配对详情。地址和中继 URL 不含配对密钥；error 保留上游原文。
home-pairing-timeout = 配对未能在 { $seconds } 秒内完成。
home-pairing-aborted = 配对没有完成就中断了，再试一次。
home-pairing-failed-detail = 直连地址：{ $addresses }。中继：[{ $relays }]。已用时 { $milliseconds } 毫秒，限时 { $seconds } 秒。原因：{ $error }
plugin-now-start-failed = 一键操作开始不了：{ $error }
