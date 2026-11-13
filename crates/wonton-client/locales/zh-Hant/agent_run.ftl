# Agent 鏈列表和鏈的動作（crates/wonton-client/src/agent_chains/）。

# 鏈列表和鏈動作要連家用伺服器，還沒連上時顯示。
agent-api-not-connected = 連上家用伺服器後再試。

# $status 是原樣的 HTTP 狀態碼。
agent-api-status = 家用伺服器傳回 HTTP { $status }

# $reason 是 JSON 解析器報的原始錯誤。
agent-api-unreadable = 讀不懂家用伺服器的回應：{ $reason }

# $reason 是 HTTP 用戶端報的原始錯誤。
agent-api-client = 無法建立 HTTP 用戶端：{ $reason }
