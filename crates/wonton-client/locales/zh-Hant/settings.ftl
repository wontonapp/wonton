# 推播方式設定（crates/wonton-client/src/push_method.rs）。
# $path 是設定檔的路徑，$reason 是原始的讀寫錯誤。
push-method-write-failed = 無法寫入推播方式設定檔 { $path }：{ $reason }

# 推播（crates/wonton-client/src/push/{mod,registration,notification}.rs）：被拒絕的位址、失敗的註冊和取不回訊息的詳細資訊。
# 通知的內文和佔位（未知的傳送者、邀請你加入、[檔案]……）還不在這裡。
# $reason 是原始錯誤詳細資訊。
push-matrix-pusher-failed = 無法在 Matrix 上註冊 pusher：{ $reason }

# $reason 是 URL 剖析器的原始錯誤。
push-endpoint-invalid-url = 推播位址不是合法的 URL：{ $reason }

push-endpoint-scheme = 推播位址必須是 http 或 https 位址

push-endpoint-p256dh = p256dh 應為 65 位元組的未壓縮 P-256 公鑰（base64url）

push-endpoint-auth = auth 應為 16 位元組（base64url）

# $reason 是上游原文，或翻譯後的逾時詳細資訊。
push-register-failed = 無法向家用伺服器註冊推播：{ $reason }

push-unregister-failed = 無法向家用伺服器取消註冊推播：{ $reason }

# $reason 是 JSON 剖析器的原始錯誤。
push-register-invalid-response = 家用伺服器傳回的推播註冊結果格式有誤：{ $reason }

# $reason 是本機代理伺服器或 HTTP 用戶端的原始錯誤。
push-invalid-proxy = 本機代理伺服器位址無效：{ $reason }

push-http-client = 無法建立 HTTP 用戶端：{ $reason }

# $seconds 是請求的逾時秒數。
push-timeout = { $seconds } 秒內沒有回應

# $status 是原始 HTTP 狀態碼。
push-server-status = 家用伺服器傳回 HTTP { $status }

# $message 和 $kind 是伺服器的原始錯誤文字和錯誤碼。
push-server-error = { $message }（{ $kind }）

# $reason 是 JSON 剖析器的原始錯誤。
push-payload-invalid = 推播內容的格式有誤：{ $reason }

# $reason 是 ID 剖析器的原始錯誤。
push-invalid-room-id = 推播裡的聊天室 ID 無效：{ $reason }

push-invalid-event-id = 推播裡的事件 ID 無效：{ $reason }

# $seconds 是取得訊息時等了的秒數。
push-fetch-timeout = { $seconds } 秒內無法取回這則訊息

# $reason 是 Matrix SDK 的原始錯誤。
push-fetch-failed = 無法取得這則訊息：{ $reason }

push-core-closed = 用戶端核心已關閉

push-not-paired = 還沒有連上過家用伺服器

push-home-unreachable = 連不上家用伺服器：{ $reason }

# $seconds 是登入時等了的秒數。
push-session-timeout = { $seconds } 秒內無法登入 Matrix

push-room-key-missing = 還沒收到這則訊息的聊天室金鑰
