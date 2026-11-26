# 中繼表設定（ADR-0048）：讀不到、存不了時的原因。
relays-not-paired = 還沒配對家裡的伺服器。

relays-not-connected = 還沒連上家裡的伺服器。

# 正在儲存中繼表，改動要等它存完。
relays-busy = 正在儲存中繼表，稍等再改。
relays-not-loaded = 中繼表還沒載入出來，先重新整理一下。

# 表由 WONTON_RELAYS 環境變數固定，這裡改不了。
relays-locked = 中繼表由環境變數固定，這裡改不了。

# $reason 是代理位址解析器的原始錯誤。
relays-invalid-proxy = 本機代理位址不合法：{ $reason }

# $reason 是 HTTP 用戶端的原始錯誤。
relays-http-client = 無法建立 HTTP 用戶端：{ $reason }

# $seconds 是請求逾時的秒數。
relays-timeout = { $seconds } 秒內沒有回應

# $reason 是上游原文，或翻譯後的逾時、協定錯誤詳情。
relays-request-failed = 連不上家裡的伺服器：{ $reason }

# $reason 是 JSON 解析器的原始錯誤。
relays-invalid-response = 家裡的伺服器給的中繼表格式不對：{ $reason }

# $status 是原始 HTTP 狀態碼。
relays-server-status = 家裡的伺服器回了 HTTP { $status }

# $message 和 $kind 是伺服器的原始錯誤說明和錯誤碼。
relays-server-error = { $message }（{ $kind }）

# $value 是原樣輸入的位址。
relays-invalid-url = 中繼位址不合法：{ $value }

# $value 是原樣輸入的位址。
relays-needs-https = 中繼位址要用 https：{ $value }

# $value 是原樣輸入的位址。
relays-has-userinfo = 中繼位址裡不能帶帳號密碼，權杖要單獨填：{ $value }

# $value 是規範後的位址。
relays-duplicate = 這一條已經在表裡了：{ $value }

# $value 是規範後的位址。
relays-missing = 表裡已經沒有這一條：{ $value }

# $value 是要插到前面的那一條規範後的位址。
relays-before-missing = 要插到前面的那一條已經不在表裡了：{ $value }

# $reason 是連線資訊解析器的原始錯誤。
relays-invalid-ticket = 儲存的連線資訊已損壞：{ $reason }
