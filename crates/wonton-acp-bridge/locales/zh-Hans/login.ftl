## Agent 登录流程的话（聊天命令、经登录控制 socket 的终端登录）。命令名不翻。

login-redact-reason = 已转交 Agent 登录
login-answer-first = 请先回复 /wonton login accept 或 /wonton login decline 处理登录链接。
login-code-limits = 授权码必须是单行文字，且不能超过 16 KiB。
login-not-sent = 登录已结束，这条消息没有发送给 Agent。
login-no-forward = 登录期间不转发普通聊天；请在官方页面完成登录，或发送 /wonton login cancel。
login-no-direct = 找不到和你的私聊，请在私聊里发 /wonton login。
login-in-direct = 登录在我们的私聊里进行。
login-cancelled = 登录已取消。
login-no-pending = 没有等待回答的登录链接。
login-finish-current = 请先结束当前操作，再发起登录。
login-starting = 正在发起 Agent 登录。取消请发送 /wonton login cancel。
login-unsupported-method = 还不支持这种登录方式。
login-unknown-method = Agent 没有提供这种登录方式；发送 /wonton login 查看。
login-secret-delete = 这条消息里有登录用的密钥，请自己删掉它。
login-done = Agent 登录完成。
login-no-methods = Agent 没有提供登录方式。
login-methods-header = 选择 Agent 登录方式：
login-method-line = { $name }：/wonton login { $id }
login-link =
    Agent 登录：
    { $message }
    { $url }
    打开此链接登录请回复 /wonton login accept；拒绝请回复 /wonton login decline。

# 错误前缀里的详情（见 errors.ftl）。
login-auth-failed = Agent 登录失败（ACP { $code }）
login-auth-kind-unsupported = 不支持 Agent 提供的登录类型
login-timeout = 登录超时，请重新发起
login-socket-connect = 连接登录控制 socket { $path } 失败：{ $error }
login-invalid-frame = 登录 socket 返回了无效帧
login-process-exited = Agent 登录进程提前退出
login-failed-detail = Agent 登录失败：{ $detail }
login-exit-code = Agent 登录进程以退出码 { $code } 失败
login-socket-closed = 登录 socket 已断开：{ $error }
login-chat-closed = 登录聊天已关闭
