## 橋送進聊天的錯誤。類別前綴在前，詳細資訊（上游原文、IO 錯誤）作參數嵌進句子，不翻。
## 前綴和下面橋自己寫的詳細資訊都不加句尾標點，詳細資訊接在前綴後面讀。台灣用詞。

err-auth-required = Agent 需要登入：傳送 /wonton login 查看登入方式。
err-not-ready = Agent 尚未就緒：{ $message }
err-transport = 傳輸無法使用：{ $message }
err-session = ACP 工作階段無法使用：{ $message }
err-upstream = 上游錯誤：{ $message }
err-invalid = 輸入無效：{ $message }
err-confirmation = 無法使用確認卡片：{ $message }
err-busy = 忙碌：{ $message }
err-matrix = Matrix 錯誤：{ $message }
err-hop-limit = 剛才幾個 Bot 在這裡來回說得太多，閘道停下了我的 { $count } 則訊息，現在已經補發。

# 橋自己寫的詳細資訊，嵌在上面這些前綴後面。
acp-no-http-mcp = Agent 沒有宣告 HTTP MCP 能力
acp-no-session-id = session/new 沒有回傳 sessionId
acp-no-steering = Agent 沒有宣告 _session/steering
acp-connection-closed = ACP 連線已關閉
acp-turn-timeout = ACP 單輪逾時，已要求取消
acp-output-limit = ACP 回覆超過輸出上限，已要求取消
acp-encode-failed = ACP 請求編碼失敗：{ $error }
acp-request-failed = ACP 請求失敗
acp-frame-too-large = ACP 請求超過單幀上限
acp-restore-unsupported = Agent 不支援恢復 ACP 工作階段

# 確認卡片（Bot 的權限確認）。
perm-mcp-url = WONTON_MCP_URL 無效：{ $error }
perm-client = 建立確認卡片 HTTP 用戶端失敗：{ $error }
perm-broker-connect = 連線確認卡片介面失敗：{ $error }
perm-broker-read = 讀取確認卡片回應失敗：{ $error }
perm-broker-timeout = 確認卡片請求逾時
perm-broker-invalid = 確認卡片回應無效：{ $error }
perm-tool-kind = ACP 工具類型：{ $kind }
perm-missing-options = ACP 權限請求缺少 options
perm-duplicate-option-id = ACP 權限請求的 optionId 重複
perm-invalid-kind = ACP 權限選項 kind 無效：{ $error }
perm-empty-options = ACP 權限請求沒有 options
perm-missing-field = ACP 權限請求缺少 { $field }
perm-redacted = [已隱藏]

# 調用鏈（ADR-0061 第 10 節）：核心不給鏈時這一輪不發給 Agent，房間裡如實說明。
chain-limit-depth = 這條對話鏈套的 Agent 層數到上限了，這一輪沒法開始。要繼續請在 Wonton 裡給這條鏈「再給一份」。
chain-limit-rounds = 這條對話鏈的輪數用完了，這一輪沒法開始。要繼續請在 Wonton 裡給這條鏈「再給一份」。
chain-limit-cost = 這條對話鏈的花費到上限了，這一輪沒法開始。要繼續請在 Wonton 裡給這條鏈「再給一份」。
chain-limit-other = 這條對話鏈用到上限了，這一輪沒法開始。要繼續請在 Wonton 裡給這條鏈「再給一份」。
chain-stopped = 這條對話鏈已經被停了，這一輪沒法開始。要繼續請重新說一句，會開一條新的鏈。
err-chain-refused = 呼叫鏈拒絕了這一輪：{ $message }
chain-unreachable = 連線呼叫鏈介面失敗：{ $error }
chain-invalid = 呼叫鏈介面的應答無效（HTTP { $status }）：{ $error }
chain-client = 建立呼叫鏈 HTTP 客戶端失敗：{ $error }
chain-url = WONTON_MCP_URL 無效：{ $error }
outbox-client = 建立出箱 HTTP 客戶端失敗：{ $error }
