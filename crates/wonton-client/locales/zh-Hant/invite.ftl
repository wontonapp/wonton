# 新增裝置時，還無法取得配對碼的原因。
invite-not-paired = 你還沒與家用伺服器配對。

# $reason 是連線資訊剖析器的原始錯誤。
invite-invalid-ticket = 儲存的連線資訊已損毀：{ $reason }

invite-not-connected = 尚未連上家用伺服器。

# $reason 是代理 URL 剖析器的原始錯誤。
invite-invalid-proxy = 本機代理伺服器位址無效：{ $reason }

# $reason 是 HTTP 用戶端的原始錯誤。
invite-http-client = 無法建立 HTTP 用戶端：{ $reason }

# $seconds 是請求逾時的秒數。
invite-timeout = { $seconds } 秒內未收到回應

# $reason 是上游原文，或翻譯後的逾時、通訊協定錯誤詳細資訊。
invite-request-failed = 無法向家用伺服器取得配對碼：{ $reason }

# $reason 是 JSON 剖析器的原始錯誤。
invite-invalid-response = 家用伺服器傳回的配對碼格式有誤：{ $reason }

# $status 是原始 HTTP 狀態碼。
invite-server-status = 家用伺服器傳回 HTTP { $status }

# $message 和 $kind 是伺服器的原始錯誤說明和錯誤碼。
invite-server-error = { $message }（{ $kind }）
