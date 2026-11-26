# 中继表设置（ADR-0048）：读不到、存不了时的原因。
relays-not-paired = 还没有配对家里的服务器。

relays-not-connected = 还没连上家里的服务器。

# 正在保存中继表，改动要等它存完。
relays-busy = 正在保存中继表，稍等再改。
relays-not-loaded = 中继表还没加载出来，先刷新一下。

# 表由 WONTON_RELAYS 环境变量固定，这里改不了。
relays-locked = 中继表由环境变量固定，这里改不了。

# $reason 是代理地址解析器的原始错误。
relays-invalid-proxy = 本地代理地址不合法：{ $reason }

# $reason 是 HTTP 客户端的原始错误。
relays-http-client = 无法创建 HTTP 客户端：{ $reason }

# $seconds 是请求超时的秒数。
relays-timeout = { $seconds } 秒内没有回应

# $reason 是上游原文，或翻译后的超时、协议错误详情。
relays-request-failed = 连不上家里的服务器：{ $reason }

# $reason 是 JSON 解析器的原始错误。
relays-invalid-response = 家里的服务器给的中继表格式不对：{ $reason }

# $status 是原始 HTTP 状态码。
relays-server-status = 家里的服务器回了 HTTP { $status }

# $message 和 $kind 是服务器的原始错误说明和错误码。
relays-server-error = { $message }（{ $kind }）

# $value 是原样输入的地址。
relays-invalid-url = 中继地址不合法：{ $value }

# $value 是原样输入的地址。
relays-needs-https = 中继地址要用 https：{ $value }

# $value 是原样输入的地址。
relays-has-userinfo = 中继地址里不能带账号密码，令牌要单独填：{ $value }

# $value 是规范后的地址。
relays-duplicate = 这一条已经在表里了：{ $value }

# $value 是规范后的地址。
relays-missing = 表里已经没有这一条：{ $value }

# $value 是要插到前面的那一条规范后的地址。
relays-before-missing = 要插到前面的那一条已经不在表里了：{ $value }

# $reason 是连接信息解析器的原始错误。
relays-invalid-ticket = 保存的连接信息已损坏：{ $reason }
