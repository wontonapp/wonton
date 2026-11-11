# $error 是 JSON 剖析器的原始錯誤。
home-ticket-invalid-json = 連線資訊不是合法的 JSON：{ $error }

# 沒有參數。掃到的連線資訊裡沒有能連上伺服器的位址。
home-ticket-no-addresses = 連線資訊裡沒有伺服器的中繼位址或直接連線位址

home-ticket-relays-invalid = 中繼表不合法：{ $reason }

# $domain 是連線資訊裡的原始基礎網域。
home-ticket-invalid-domain = 基礎網域無效：{ $domain }

# $error 是位址剖析器的原始錯誤。
home-ticket-invalid-web-url = 應用程式網頁位址無效：{ $error }

# $domain 是連線資訊的基礎網域，$url 是連線資訊裡的原始位址。
home-ticket-web-url-under-domain = 應用程式網頁位址必須是 { $domain } 下的 http(s) 位址：{ $url }

# $error 是位址剖析器的原始錯誤。
home-ticket-invalid-core-url = 家用伺服器位址無效：{ $error }

# $domain 是連線資訊的基礎網域，$url 是連線資訊裡的原始位址。
home-ticket-core-url-under-domain = 家用伺服器位址必須是 { $domain } 的 http(s) 位址：{ $url }

# $error 是網路初始化的原始錯誤。
# 沒有參數。配對頁的裝置名欄位被清空了。
home-pairing-name-empty = 裝置名是空的。

home-pairing-bind-failed = 無法建立網路連線：{ $error }

# $reason 是連線資訊剖析器的原始錯誤。
home-ticket-corrupt = 儲存的連線資訊已損毀：{ $reason }

home-no-connection-info = 還沒有家用伺服器的連線資訊

# $error 是本機代理原始的繫結錯誤。
home-proxy-start-failed = 本機代理啟動失敗：{ $error }

home-tunnel-closed = 隧道用戶端已關閉

home-lan-unsupported = 這個版本的用戶端核心不支援區域網路設定

home-port-mapping-save-failed = 連接埠映射的開關存不下來：{ $error }

# 連線的本機儲存空間（crates/wonton-client/src/home/storage.rs）。
# $path 是檔案的路徑，$reason 是原始的剖析或讀寫錯誤。
home-storage-device-key-corrupt = 裝置金鑰檔案 { $path } 已損毀：{ $reason }

home-storage-io-failed = 無法讀寫 { $path }：{ $reason }

# 配對詳細資訊。位址和中繼 URL 不含配對密鑰；error 保留上游原文。
home-pairing-timeout = 配對未能在 { $seconds } 秒內完成。

home-pairing-aborted = 配對沒有完成就中斷了，再試一次。

home-pairing-not-pairing = 沒有正在進行的配對。

home-pairing-failed-detail = 直連位址：{ $addresses }。中繼：[{ $relays }]。已用時 { $milliseconds } 毫秒，時限 { $seconds } 秒。原因：{ $error }
