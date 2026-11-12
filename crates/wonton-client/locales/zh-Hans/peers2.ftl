# $seconds 是请求超时的秒数。
peers-timeout = { $seconds } 秒内没有回应

# $status 是原始 HTTP 状态码。
peers-server-status = 家里的服务器回了 HTTP { $status }

# $reason 是隧道拒绝的原话。
peers-rejected = 家里的服务器拒绝了这台设备：{ $reason }
