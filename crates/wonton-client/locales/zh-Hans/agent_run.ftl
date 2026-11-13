# Agent 链列表和链的动作（crates/wonton-client/src/agent_chains/）。

# 链列表和链动作要连家里的服务器，还没连上时显示。
agent-api-not-connected = 连上家里的服务器后再试。

# $status 是原样的 HTTP 状态码。
agent-api-status = 核心回了 { $status }

# $reason 是 JSON 解析器报的原始错误。
agent-api-unreadable = 读不懂家里服务器的回应：{ $reason }

# $reason 是 HTTP 客户端报的原始错误。
agent-api-client = 建 HTTP 客户端失败：{ $reason }
