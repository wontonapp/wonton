chat-sign-in-no-connection-info = 還沒有家用伺服器的連線資訊

chat-sign-in-no-saved-session = 登入後取不到登入狀態

# 取得登入權杖的請求。$reason 是上游原文，或翻譯後的逾時詳細資訊。
# $reason 是代理位址剖析器的原始錯誤。
chat-login-invalid-proxy = 本機代理伺服器位址無效：{ $reason }

chat-login-request-failed = 無法向家用伺服器取得 Matrix 登入權杖：{ $reason }

# $message 和 $kind 是伺服器的原始錯誤說明和錯誤碼。
chat-login-server-error = { $message }（{ $kind }）

# 恢復相關。
chat-recovery-not-connected = 還沒連上家用伺服器

chat-recovery-reset-no-response = 家用伺服器沒有回應，備份沒有動，稍後再試

chat-recovery-empty-key = 復原金鑰不能為空

# 回答或停止時段沒成功時，確認卡片下方問題詳細資訊裡顯示的話。
# $message 是家用伺服器的原始錯誤說明，或翻譯後的詳細資訊。
chat-confirmation-answer-failed = 回答沒有成功：{ $message }

# $error 是 HTTP 用戶端的原始錯誤。
chat-confirmation-answer-lost = 沒等到家用伺服器的回應，不確定是否已答上，可以再點一次（{ $error }）

# $message 是家用伺服器的原始錯誤說明，或翻譯後的詳細資訊。
chat-confirmation-stop-failed = 沒有停下來：{ $message }

# $error 是 JSON 剖析器的原始錯誤。
chat-confirmation-response-unreadable = 讀不懂家用伺服器的回應（{ $error }）

# $status 是原始 HTTP 狀態碼。
chat-confirmation-core-status = 家用伺服器傳回 HTTP { $status }

# $reason 是 Matrix ID 剖析器的原始錯誤。
chat-contacts-invalid-user-id = 使用者 ID 無效：{ $reason }

chat-layout-invalid-bot-id = ID 的格式不對，應該像 @名字:伺服器。

chat-layout-change-failed = 改顯示方式沒成功：{ $reason }

# Matrix 撤銷本裝置、帳號標記為失敗時顯示。
chat-session-device-revoked = 這台裝置在 Matrix 上已被撤銷，需要主人重新配對。

# 發起私聊時給的 ID 不像 @名稱:伺服器時顯示。
chat-invalid-user-id = ID 格式不對，應該像 @名稱:伺服器

# $file_type 是所選檔案原始的 MIME 類型。
chat-not-image-type = 這個檔案不是圖片：{ $file_type }

# 還沒連上家用伺服器就回答或停止確認時顯示。
chat-action-not-connected = 還沒連上家用伺服器

# $error 是房間 ID 剖析器的原始錯誤。
chat-invalid-room-id = 聊天室 ID 無效：{ $error }

# $room 是原始的房間 ID。
chat-room-not-subscribed = 要先訂閱聊天室 { $room } 的時間軸

# 還沒選預設 Agent 就詢問時顯示。
chat-no-default-agent = 還沒有預設 Agent：先在設定裡選一個

chat-op-display-name-failed = 無法同步顯示名稱：{ $error }

chat-op-resend-failed = 無法重新傳送訊息：{ $error }

chat-op-send-image-failed = 無法傳送圖片：{ $error }

chat-op-forward-failed = 無法轉傳訊息：{ $error }

chat-op-redact-failed = 無法收回訊息：{ $error }

chat-avatar-cache-not-file = 頭像快取路徑不是檔案

chat-avatar-cache-missing-mxc = 頭像快取缺少 MXC 位址

chat-avatar-cache-path-not-utf8 = 頭像快取路徑不是有效的 UTF-8

# 聊天的本機儲存空間（crates/wonton-client/src/chat/storage.rs）。
# $path 是檔案的路徑，$reason 是原始的剖析或讀寫錯誤。
chat-storage-session-corrupt = 聊天登入狀態檔案 { $path } 已損毀：{ $reason }

chat-storage-io-failed = 無法讀寫 { $path }：{ $reason }

# 聊天的連線問題（crates/wonton-client/src/chat/view.rs）：CoreError::TransportUnavailable 的原因和失敗操作的訊息。
chat-not-connected = 連上家用伺服器後再試

chat-draft-save-failed = 存草稿失敗：{ $error }

chat-draft-not-signed-in = 還沒有登入 Matrix

# 確認卡片下方放行時段結束後的那一行。$count 是開啟時段那次之外放行了幾次，0 表示不知道或沒有。
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] 放行時段已到期。
               *[other] 放行時段已到期，放行了 { $count } 次。
            }
        [stopped]
            { $count ->
                [0] 放行時段已停止。
               *[other] 放行時段已停止，放行了 { $count } 次。
            }
        [restarted] 伺服器重新啟動過，放行時段已結束。
       *[other]
            { $count ->
                [0] 放行時段已結束。
               *[other] 放行時段已結束，放行了 { $count } 次。
            }
    }
