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

# $path is the private device file; $reason is the underlying storage error.
notification-lock-screen-storage-failed = 读写不了锁屏内容设置 { $path }：{ $reason }
