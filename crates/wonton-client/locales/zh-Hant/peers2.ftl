# $seconds 是請求逾時的秒數。
peers-timeout = { $seconds } 秒內沒有回應

# $status 是原始 HTTP 狀態碼。
peers-server-status = 家裡的伺服器回了 HTTP { $status }

# $reason 是隧道拒絕的原話。
peers-rejected = 家裡的伺服器拒絕了這台裝置：{ $reason }
