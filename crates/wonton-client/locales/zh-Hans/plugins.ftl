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
files-playback-transport = 连不上家里的服务器。

# 这台设备放不了这个格式时显示。
files-playback-unsupported = 这台手机放不了这个格式。可以下载后用其他 App 打开。

# $detail 是家里的服务器或连接的原始原因。
files-playback-unavailable = 家里的服务器给不了这个文件：{ $detail }

# $detail 是家里的服务器或连接的原始原因。
files-playback-error = 家里的服务器报错：{ $detail }。

# 播放探测超时时显示。
files-playback-no-response = 核心一直没有回答

# $reason 是原始 io 错误。
files-stream-local-address = 开不了播放用的本地地址：{ $reason }

# 隧道没连上时播放失败的诊断详情。
files-stream-not-connected = 隧道没连上

# $code 是隧道原始错误码。
files-stream-tunnel-unavailable = 隧道不可用（{ $code }）

# 底栏 ViewModel 的错误，作为 CoreError 的 reason 或底栏的问题提示显示。
# 无参数。
tabs-view-always-pinned = 聊天和应用已经固定到底栏。

tabs-view-plugin-list-not-ready = 插件列表还没读到，暂时不能操作插件页面。

tabs-view-unknown-plugin = 没有这个插件。

# $error 是本地存储的原始错误。
tabs-storage-problem = 底栏设置存不下，下次启动可能恢复旧状态：{ $error }

tabs-view-plugin-no-page = 插件没有可打开的网页。

tabs-page-not-placed = 这个页面不在面板里。

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

# 会话列表（crates/wonton-client/src/session_store.rs）。
sessions-title-required = 会话标题不能为空

# $id 是原样的会话编号。
sessions-not-found = 会话不存在：{ $id }

# 壳把不认识的操作分给会话时显示。
sessions-invalid-action = 不是会话操作

# 应用页面的桥（crates/wonton-client/src/tabs/bridge/）。
tabs-bridge-page-not-open = 这个插件页面没有打开或固定

tabs-bridge-invalid-environment = 页面栏高和安全区必须是有限的非负数

tabs-bridge-public-page = 公网网页没有 Wonton 的桥。

tabs-page-cannot-reload = 这个页面不能重新加载。

tabs-bar-not-pinned = 这个页面没有固定到底栏。

tabs-bar-already-pinned = 这个页面已经固定到底栏。

tabs-bar-replace-required = 底栏已满，必须指定要替换的页面。

tabs-bar-replacing-missing = 要替换的页面不在底栏。

tabs-bar-fixed = 聊天和应用不能被替换。

tabs-bar-not-full = 底栏未满时不能指定替换页面。

tabs-bar-remove-fixed = 聊天和应用不能拿掉。

tabs-bar-page-missing = 这个页面不在底栏。

tabs-bar-index-out-of-range = 底栏下标越界。

tabs-open-only-plugins = 只有插件页面可以设置打开方式。

tabs-fullscreen-only-plugins = 只有插件页面可以全屏。

tabs-fullscreen-none = 当前没有全屏页面。

tabs-fullscreen-not-current = 退出的页面不是当前全屏页面。
