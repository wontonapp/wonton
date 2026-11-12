## 默认收起的技术详情。原因、错误码和上游原文保持不变。
## 不限长度。参数是诊断数据，不是翻译键。

# CoreError 前缀。$reason 是原始错误详情。
core-error-storage = 本机存储读写失败：{ $reason }
core-error-not-ready = 尚未就绪：{ $reason }
core-error-transport = 连不上家里的服务器：{ $reason }
core-error-unauthorized = 家里的服务器拒绝了请求：{ $reason }
core-error-upstream = 家里的服务器出错：{ $reason }

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

# 通过 Siri 或快捷指令发消息、内容为空时显示。
chat-ask-empty-message = 消息不能为空。
# Siri 或快捷指令传进来的 Bot ID 解析不了时显示。
chat-ask-invalid-bot-id = 默认 Agent 的 ID 格式不对。
# 已有私聊的房间 ID 解析不了时显示。
chat-ask-invalid-room-id = 私聊的房间 ID 不合法。

# Matrix 吊销本设备、账号标记为失败时显示。
chat-session-device-revoked = 这台设备在 Matrix 上已被吊销，需要主人重新配对。
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
home-pairing-bind-failed = 绑定网络失败：{ $error }
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
tabs-config-missing-fixed = 底栏缺少聊天或应用。
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
