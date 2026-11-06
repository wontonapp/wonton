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
