# Agent のチェーンリストとチェーンの操作（crates/wonton-client/src/agent_chains/）。

# チェーンリストやチェーンの操作には自宅サーバーへの接続が必要です。未接続のときに表示します。
agent-api-not-connected = 自宅サーバーに接続してからもう一度お試しください。

# $status はそのままの HTTP ステータスコードです。
agent-api-status = 自宅サーバーからHTTP { $status }が返されました

# $reason は JSON パーサーの元のエラーです。
agent-api-unreadable = 自宅サーバーからの応答を読み取れません：{ $reason }

# $reason は HTTP クライアントの元のエラーです。
agent-api-client = HTTPクライアントを作成できません：{ $reason }
