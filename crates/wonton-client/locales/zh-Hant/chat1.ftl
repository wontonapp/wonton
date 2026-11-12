## chat-a 片段：crates/wonton-client/src/chat/{login,sign_in,recovery}.rs。
## 是聊天問題/錯誤預設收合的「詳細資訊」：CoreError 的原因和 ChatProblem 的 message。
# 登入相關。$reason 是 Matrix SDK 的原始錯誤詳細資訊。
chat-sign-in-restore-failed = 無法恢復 Matrix 工作階段：{ $reason }

chat-sign-in-external-missing = 這支手機上已經沒有這個 Matrix 帳號的登入狀態，請重新登入。

chat-sign-in-login-failed = 無法登入 Matrix：{ $reason }

# $reason 是 HTTP 用戶端的原始錯誤。
chat-login-http-client = 無法建立 HTTP 用戶端：{ $reason }

# $seconds 是請求逾時的秒數。
chat-login-timeout = { $seconds } 秒內未收到回應

# $reason 是讀取 HTTP 回應的原始錯誤。
chat-login-response-failed = 無法讀取家用伺服器的回應：{ $reason }

# $reason 是 JSON 剖析器的原始錯誤。
chat-login-invalid-token = 家用伺服器傳回的登入權杖格式有誤：{ $reason }

# $login_type 是家用伺服器給的登入方式；$expected 是要求的登入方式。
chat-login-wrong-type = 家用伺服器給的登入方式 { $login_type } 不是 { $expected }

# $status 是原始 HTTP 狀態碼。
chat-login-server-status = 家用伺服器傳回 HTTP { $status }

# $reason 是儲存的原始錯誤詳細資訊。
chat-recovery-key-not-saved = 復原金鑰無法儲存到本機，請現在就抄下來：{ $reason }

# $error 是 HTTP 用戶端的原始錯誤。
chat-confirmation-stop-lost = 沒等到家用伺服器的回應，可以再點一次（{ $error }）

# 開始私聊時使用者名稱為空時顯示。
chat-contacts-empty-username = 使用者名稱不能為空。

# 透過 Siri、捷徑或聊天頁傳送訊息、內容為空時顯示。
chat-empty-message = 訊息不能為空。

# Siri 或捷徑傳入的 Bot ID 剖析失敗時顯示。
chat-ask-invalid-bot-id = 預設 Agent 的 ID 格式不對。

# 已有私訊的聊天室 ID 剖析失敗時顯示。
chat-ask-invalid-room-id = 私訊的聊天室 ID 無效。

chat-invalid-action = 不是聊天操作

# 殼送出這個核心不認得的回答時顯示。
chat-unknown-answer = 這個版本的雲吞不認得這個回答

# 還沒登入 Matrix 就有聊天操作時顯示。
chat-not-signed-in = 還沒登入 Matrix

chat-send-no-response = Matrix 伺服器沒有回應

chat-link-room-not-joined = 還沒加入這個聊天室，請先在聊天列表接受邀請。

# 在只放通知的房間裡送東西時顯示。
chat-read-only-room = 這個聊天室只放通知，不能傳送訊息

# $error 是事件 ID 解析器的原始錯誤。
chat-invalid-event-id = 訊息的事件 ID 無效：{ $error }

# 要轉傳或收回的訊息選單裡沒有這一項（已收回、還沒傳出去、沒有權限等）時顯示。
chat-message-action-unavailable = 這則訊息不能這樣操作

# 轉傳的目標聊天室不存在或還沒加入時顯示。
chat-forward-target-not-joined = 還沒有加入要轉傳到的聊天室

# 背景聊天操作失敗的訊息，放進問題提示；$error 是 Matrix SDK 或家用伺服器的原始錯誤。
chat-op-ask-agent-failed = 無法詢問預設 Agent：{ $error }

chat-op-join-failed = 無法加入聊天室：{ $error }

chat-op-leave-failed = 無法離開聊天室：{ $error }

chat-op-cancel-send-failed = 無法取消傳送：{ $error }

chat-op-paginate-failed = 無法載入更早的訊息：{ $error }

chat-op-mark-read-failed = 無法標為已讀：{ $error }

chat-op-direct-chat-failed = 無法發起私訊：{ $error }

chat-op-send-text-failed = 無法傳送訊息：{ $error }

chat-avatar-cache-hash-collision = 頭像快取的雜湊值衝突

chat-draft-clear-failed = 清掉已發出的草稿失敗：{ $error }

chat-draft-read-interrupted = 草稿的讀取被打斷了
