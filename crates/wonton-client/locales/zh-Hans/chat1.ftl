## chat-a 片段：crates/wonton-client/src/chat/{login,sign_in,recovery}.rs。
## 是聊天问题/错误默认收起的“详情”：CoreError 的原因和 ChatProblem 的 message。
# 登录相关。$reason 是 Matrix SDK 的原始错误。
chat-sign-in-restore-failed = 恢复 Matrix 会话失败：{ $reason }

chat-sign-in-external-missing = 这台手机上已经没有这个 Matrix 账号的登录状态，请重新登录。

chat-sign-in-login-failed = 登录 Matrix 失败：{ $reason }

# $reason 是 HTTP 客户端的原始错误。
chat-login-http-client = 建 HTTP 客户端失败：{ $reason }

# $seconds 是请求超时的秒数。
chat-login-timeout = { $seconds } 秒内没有回应

# $reason 是读 HTTP 回应的原始错误。
chat-login-response-failed = 读核心的回应失败：{ $reason }

# $reason 是 JSON 解析器的原始错误。
chat-login-invalid-token = 核心给的登录令牌格式不对：{ $reason }

# $login_type 是核心给的登录方式；$expected 是要求的登录方式。
chat-login-wrong-type = 核心给的登录方式 { $login_type } 不是 { $expected }

# $status 是原始 HTTP 状态码。
chat-login-server-status = 核心回了 { $status }

# $reason 是存储的原始错误。
chat-recovery-key-not-saved = 恢复密钥没能存到本机，请现在就抄下来：{ $reason }

# $error 是 HTTP 客户端的原始错误。
chat-confirmation-stop-lost = 没等到家里服务器的回应，可以再点一次（{ $error }）

# 开始私聊时用户名为空时显示。
chat-contacts-empty-username = 用户名不能为空。

# 通过 Siri、快捷指令或聊天页发消息、内容为空时显示。
chat-empty-message = 消息不能为空。

# Siri 或快捷指令传进来的 Bot ID 解析不了时显示。
chat-ask-invalid-bot-id = 默认 Agent 的 ID 格式不对。

# 已有私聊的房间 ID 解析不了时显示。
chat-ask-invalid-room-id = 私聊的房间 ID 不合法。

# 壳把不认识的操作分给聊天时显示。
chat-invalid-action = 不是聊天操作

# 壳给了这个核心不认识的回答时显示。
chat-unknown-answer = 这个版本的 App 不认识这个回答

# 还没登录 Matrix 就有聊天操作时显示。
chat-not-signed-in = 还没有登录 Matrix

chat-send-no-response = Matrix 服务器没有回应

# 往只放通知的房间里发东西时显示。
chat-read-only-room = 这个房间只放通知，不能发消息

# $error 是事件 ID 解析器的原始错误。
chat-invalid-event-id = 消息的事件 ID 不合法：{ $error }

# 要转发或撤回的消息菜单里没有这一项（已撤回、还没发出去、没有权限等）时显示。
chat-message-action-unavailable = 这条消息不能这样操作

# 转发的目标房间不存在或还没加入时显示。
chat-forward-target-not-joined = 还没有加入要转发到的房间

# 后台聊天操作失败的话，放进问题提示里；$error 是 Matrix SDK 或家里的服务器的原始错误。
chat-op-ask-agent-failed = 问默认 Agent 失败：{ $error }

chat-op-join-failed = 加入房间失败：{ $error }

chat-op-leave-failed = 离开房间失败：{ $error }

chat-op-cancel-send-failed = 取消发送失败：{ $error }

chat-op-paginate-failed = 加载更早的消息失败：{ $error }

chat-op-mark-read-failed = 标为已读失败：{ $error }

chat-op-direct-chat-failed = 发起私聊失败：{ $error }

chat-op-send-text-failed = 发送消息失败：{ $error }

chat-avatar-cache-hash-collision = 头像缓存哈希冲突

chat-draft-clear-failed = 清掉已发出的草稿失败：{ $error }

chat-draft-read-interrupted = 草稿的读取被打断了
