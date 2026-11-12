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
