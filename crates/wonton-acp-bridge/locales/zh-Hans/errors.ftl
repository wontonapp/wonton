## 桥发进聊天看的错误。类别前缀在前，详情（上游原文、IO 错误）作参数嵌进句子，不翻。
## 前缀和下面桥自己写的详情都不带句末标点，详情接在前缀后面读。

err-auth-required = Agent 需要登录：发送 /wonton login 查看登录方式。
err-not-ready = Agent 未就绪：{ $message }
err-transport = 传输不可用：{ $message }
err-session = ACP 会话不可用：{ $message }
err-upstream = 上游错误：{ $message }
err-invalid = 输入非法：{ $message }
err-confirmation = 确认卡片不可用：{ $message }
err-busy = 忙：{ $message }
err-matrix = Matrix 错误：{ $message }
err-hop-limit = 刚才几个 Bot 在这里来回说得太多，网关停下了我的 { $count } 条消息，现在已经补发。

# 桥自己写的详情，嵌在上面这些前缀后面。
acp-no-http-mcp = Agent 未声明 HTTP MCP 能力
acp-no-session-id = session/new 没有返回 sessionId
acp-no-steering = Agent 未声明 _session/steering
acp-connection-closed = ACP 连接已关闭
acp-turn-timeout = ACP 单轮超时，已请求取消
acp-output-limit = ACP 回复超过输出限制，已请求取消
acp-encode-failed = ACP 请求编码失败：{ $error }
acp-request-failed = ACP 请求失败
acp-frame-too-large = ACP 请求超过单帧限制
acp-restore-unsupported = Agent 不支持恢复 ACP session

# 确认卡片（Bot 的权限确认）。
perm-mcp-url = WONTON_MCP_URL 无效：{ $error }
perm-client = 建立确认卡片 HTTP 客户端失败：{ $error }
perm-broker-connect = 连接确认卡片接口失败：{ $error }
perm-broker-read = 读取确认卡片响应失败：{ $error }
perm-broker-timeout = 确认卡片请求超时
perm-broker-invalid = 确认卡片响应无效：{ $error }
perm-tool-kind = ACP 工具类型：{ $kind }
perm-missing-options = ACP 权限请求缺少 options
perm-duplicate-option-id = ACP 权限请求的 optionId 重复
perm-invalid-kind = ACP 权限选项 kind 无效：{ $error }
perm-empty-options = ACP 权限请求没有 options
perm-missing-field = ACP 权限请求缺少 { $field }
perm-redacted = [已隐藏]
