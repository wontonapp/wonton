## 文件服务发给 App、Bot 和 Agent 的文字（ADR-0043）。一条完整的话一条消息，不在代码里拼；
## 数据（文件编号、大小、插件 ID、上游给的原因）作参数嵌进句子，不翻。
## 简体中文是现有的说法；对用户叫“应用”，不叫“插件”；不对用户说“核心”，说“家里的服务器”。

files-not-found = 没有这个文件
files-moved-or-deleted = 文件已被移动或删除
files-not-shared = 这个文件没有分享给你
files-share-expired = 这个编号的分享过期了，再找一次文件
files-changed = 文件登记后改过，你看过的可能不是现在的内容，再找一次
files-share-not-authorized = 应用 { $plugin } 没有共享文件夹 { $share } 的授权
files-mcp-gone = 应用 { $plugin } 在运行，但不再提供 MCP 服务，读不了这个资源
files-plugin-unavailable = 来源应用 { $plugin } 卸了或没在运行
files-notify-no-matrix = 家里的服务器没有接入 Matrix（WONTON_MATRIX_URL），发不到“Wonton”私聊
files-matrix-send-failed = 发不到“Wonton”私聊：{ $error }
files-upstream-not-own = { $service } 不是应用 { $plugin } 自己的服务

# 预览和缩略图。
files-too-big-for-thumbnail = 文件超过 { $limit } MiB，不做缩略图
files-pdf-unreadable = 这个 PDF 解析不了（可能加密了或结构坏了），读不了内容
files-pdf-no-text = 这个 PDF 没有文字层（是扫描件或整页图），共 { $pages } 页，看不到图表里的内容
files-too-big-for-parsing = 文件超过 { $limit } MiB，抽不出内容
files-video-unsupported = 这个视频的封装读不了（只认 MP4、M4V 和 MOV），只能下载或分享
files-video-unreadable = 这个视频解析不了（封装或结构不认识），拿不到时长和分辨率
files-audio-unreadable = 音频读不了内容，只能下载或分享
files-thumbnail-timeout = 原文件 { $after } 内没有读完
files-idle-limited = 应用的响应 { $idle } 没有新的字节
files-declared-size-over-limit = 应用声明的大小超过 resources/read 的上限（{ $limit } MiB）
files-not-text-nul = { $name } 不是文本文件（内容里有 NUL 字节），读不了内容：PDF 等二进制文件这类只能下载或分享。
files-not-text-utf8 = { $name } 不是文本文件：内容解不成 UTF-8，读不了内容。PDF 等二进制文件这类只能下载或分享。
files-no-preview = 这个类型没有预览或缩略图
files-preview-failed = 做不了预览或缩略图：{ $why }
files-decode-busy = 图片解码忙
files-view-busy = 这个文件正在读，稍后再试
files-read-busy = 图片读取忙

# 存文件、上传和删除。
files-stage-no-space = 放不下这 { $total } 字节，磁盘要留 5% 空闲
files-delete-transfers-only = 家里的服务器只删共享文件夹 { $folder } 里的文件
files-size-mismatch = 传完的数据有 { $actual } 字节，创建时声明的是 { $size } 字节
files-no-hard-links = { $name }：共享文件夹 { $folder } 所在的文件系统不支持硬链接
files-name-taken = { $preferred } 已经有 { $count } 个重名的文件
files-short-read = 只读到 { $copied } 字节，应该有 { $size } 字节
files-upload-disk-full = 磁盘放不下这个 { $size } 字节的文件（要留 { $percent }% 空闲，inode 也不能用完）
files-upload-leftover = 磁盘只剩 { $free } 字节，这个上传还差 { $left } 字节
files-upload-quota-full = 磁盘配额满了，这个上传放不下
files-upload-inodes-full = 磁盘的 inode 用完了，这个上传放不下
files-upload-record-label = 待完成的上传记录
files-upload-record-unwritable = 待完成的上传记录没能落盘，先不放
files-path-collision = 这个路径已经是别的文件了：编号删了，文件没动
files-residual-undeletable = 清不掉这个上传的残留，文件没有删

# 读应用资源用的名额。
files-plugin-read-overrun = 应用 { $plugin } 有读取超过 { $seconds } 秒没有结束
files-slots-full = 应用 { $plugin } 的读取排满了，稍后再试
files-plugin-slots-exhausted = 应用资源的读取都占满了

# 从应用读资源。
files-resource-invalid-url = 内网地址不合法：{ $error }
files-resource-http-only = 内网地址只能用 http
files-resource-no-credentials = 内网地址不能带用户名或密码
files-resource-host = 内网地址的主机名要是应用的服务名，不用 IP 地址
files-resource-unreachable = 连不上应用的服务：{ $why }
files-resource-redirect = 应用的内网地址返回了跳转（{ $status }），家里的服务器不跟随
files-upstream-status = 应用返回 { $status }：{ $detail }
files-upstream-read-failed = 读应用的响应失败：{ $error }

# 删文件用的回收槽。
files-trash-foreign-slot = 回收槽 { $name } 不是家里的服务器建的（属主 { $owner }、权限 { $mode }，应该是 { $euid }、700），要重启家里的服务器才能再删
files-trash-placeholder-missing = 回收槽 { $index } 的占位文件不见了，要重启家里的服务器才能再删
files-trash-full = { $folder } 里的回收槽都被占着，重启家里的服务器找回里面的文件后才能再删

# 共享文件夹。
files-check-share = 没有共享文件夹 { $share }，用 wonton__list_shares 看有哪些

# 网关的文件接口。
files-service-disabled = 家里的服务器没有启用文件服务
files-device-only = 文件接口只给经隧道的已配对设备，还要带 { $header }: 1
files-stream-shortened = 文件在下载途中变短了
files-invalid-room-body = 请求体应为 {"{"}"room": …{"}"}：{ $error }

# 经 tusd 的上传。
files-tusd-unconfigured = 家里的服务器没有配置 tusd（WONTON_TUSD_URL）
files-tusd-required = 上传接口要带 Tus-Resumable: 1.0.0
files-tusd-timeout = tusd 迟迟没有回应，稍后再试
files-tusd-gone = tusd 已经没有这个上传的数据了，要重传
files-upload-disconnected = 创建这个上传的请求已经断开了
files-upload-length-required = 上传要在创建时声明长度（Upload-Length）
files-upload-filename-required = 上传要在 Upload-Metadata 里带 filename
files-upload-finished = 这个上传已经传完了；要删文件用 DELETE /_wonton/files/{ $id }
files-upload-other-device = 这个上传是另一台设备创建的
files-upload-stale = 这个上传很久没有进度，已经清掉了，要重传
files-hook-tusd-only = 文件钩子只给 tusd 用
files-hook-not-from-gateway = 这个请求没有经过网关
