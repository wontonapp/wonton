chat-sign-in-no-connection-info = 还没有家里服务器的连接信息

chat-sign-in-no-saved-session = 登录后取不到登录状态

# 要登录令牌的请求。$reason 是上游原文，或翻译后的超时详情。
# $reason 是代理地址解析器的原始错误。
chat-login-invalid-proxy = 本地代理地址不合法：{ $reason }

chat-login-request-failed = 向核心要 Matrix 登录令牌失败：{ $reason }

# $message 和 $kind 是服务器的原始错误说明和错误码。
chat-login-server-error = { $message }（{ $kind }）

# 恢复相关。
chat-recovery-not-connected = 还没连上家里的服务器

chat-recovery-reset-no-response = 家里的服务器没有回应，备份没有动，稍后再试

chat-recovery-empty-key = 恢复密钥不能为空

# 回答或停止时段没成功时，卡片下面问题详情里显示的话。
# $message 是核心的原始错误说明，或翻译后的详情。
chat-confirmation-answer-failed = 回答没有成功：{ $message }

# $error 是 HTTP 客户端的原始错误。
chat-confirmation-answer-lost = 没等到家里服务器的回应，不确定答上了没有，可以再点一次（{ $error }）

# $message 是核心的原始错误说明，或翻译后的详情。
chat-confirmation-stop-failed = 没有停下来：{ $message }

# $error 是 JSON 解析器的原始错误。
chat-confirmation-response-unreadable = 核心的回应读不懂（{ $error }）

# $status 是原始 HTTP 状态码。
chat-confirmation-core-status = 核心回了 { $status }

# $reason 是 Matrix ID 解析器的原始错误。
chat-contacts-invalid-user-id = 用户 ID 不合法：{ $reason }

chat-layout-invalid-bot-id = ID 的格式不对，应该像 @名字:服务器。

chat-layout-change-failed = 改显示方式没成功：{ $reason }

# Matrix 吊销本设备、账号标记为失败时显示。
chat-session-device-revoked = 这台设备在 Matrix 上已被吊销，需要主人重新配对。

# 发起私聊时给的 ID 不像 @名字:服务器 时显示。
chat-invalid-user-id = ID 的格式不对，应该像 @名字:服务器

# $file_type 是选中文件原始的 MIME 类型。
chat-not-image-type = 这个文件不是图片：{ $file_type }

# 还没连上家里的服务器就回答或停止确认时显示。
chat-action-not-connected = 还没连上家里的服务器

# $error 是房间 ID 解析器的原始错误。
chat-invalid-room-id = 房间 ID 不合法：{ $error }

# $room 是原始的房间 ID。
chat-room-not-subscribed = 要先订阅房间 { $room } 的时间线

# 还没选默认 Agent 就问它时显示。
chat-no-default-agent = 还没有默认 Agent：先在设置里选一个

chat-op-display-name-failed = 同步显示名失败：{ $error }

chat-op-resend-failed = 重发消息失败：{ $error }

chat-op-send-image-failed = 发送图片失败：{ $error }

chat-op-forward-failed = 转发消息失败：{ $error }

chat-op-redact-failed = 撤回消息失败：{ $error }

chat-avatar-cache-not-file = 头像缓存路径不是文件

chat-avatar-cache-missing-mxc = 头像缓存缺少 MXC 标识

chat-avatar-cache-path-not-utf8 = 头像缓存路径不是有效的 UTF-8

# 聊天的本机存储（crates/wonton-client/src/chat/storage.rs）。
# $path 是文件的路径，$reason 是原始的解析或读写错误。
chat-storage-session-corrupt = 聊天的登录状态 { $path } 已损坏：{ $reason }

chat-storage-io-failed = 读写不了 { $path }：{ $reason }

# 聊天的连接问题（crates/wonton-client/src/chat/view.rs）：CoreError::TransportUnavailable 的原因和失败操作的消息。
chat-not-connected = 连上家里的服务器后再试

chat-draft-save-failed = 存草稿失败：{ $error }

chat-draft-not-signed-in = 还没有登录 Matrix

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
