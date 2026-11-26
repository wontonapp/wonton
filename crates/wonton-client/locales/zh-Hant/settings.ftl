# 推播方式設定（crates/wonton-client/src/push_method.rs）。
# $path 是設定檔的路徑，$reason 是原始的讀寫錯誤。
push-method-write-failed = 無法寫入推播方式設定檔 { $path }：{ $reason }

# 將代理 TUN 的假位址排除出選路的設定（crates/wonton-client/src/home/path_settings.rs）。
# $path 是設定檔的路徑，$reason 是原始的讀寫錯誤。
path-settings-write-failed = 無法寫入選路設定檔 { $path }：{ $reason }

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

# 附件通知上的文字（ADR-0118）：類型和檔名。
push-preview-image = 圖片 { $filename }
push-preview-file = 檔案 { $filename }

push-core-closed = 用戶端核心已關閉

push-not-paired = 還沒有連上過家用伺服器

push-home-unreachable = 連不上家用伺服器：{ $reason }

# $seconds 是登入時等了的秒數。
push-session-timeout = { $seconds } 秒內無法登入 Matrix

push-room-key-missing = 還沒收到這則訊息的聊天室金鑰

# $path is the private device file; $reason is the underlying storage error.
notification-lock-screen-storage-failed = 無法讀寫鎖屏內容設定 { $path }：{ $reason }

# Privacy page (crates/wonton-client/src/about/mod.rs).
privacy-what-title = 沒有雲
privacy-what-own = 雲吞沒有自己的雲：伺服器就是你家（或你自己租的）那台機器，訊息、照片、檔案都存在它上面。
privacy-what-none = 我們收不到帳號、裝置資訊，App 裡也沒有統計、廣告和當機回報。

privacy-messages-title = 聊天和檔案
privacy-messages-e2ee = 聊天和檔案都端對端加密，能解開的只有你的裝置。
privacy-messages-history = 歷史存在你自己的伺服器上，備份金鑰由你保管。

privacy-push-title = 通知（僅 Android）
privacy-push-keepalive = 用「保持連線」時，通知不走外面，隧道的另一頭就是你家。
privacy-push-unified = 改用 UnifiedPush 後，家裡的 Core 先用你裝置的公鑰把通知加密，再交給分發器；分發器只看得到密文，到手機上才解開。分發器是第三方服務。

privacy-network-title = 怎麼連上家裡
privacy-network-lan = 同一個區域網路時直連；不在一個網路時先打洞，直連不通才經中繼轉發。
privacy-network-relay = 中繼只轉發加密後的流量，看不到內容。預設用 iroh 的公共中繼，你家可以換成自己的。

privacy-preview-title = 連結預覽
privacy-preview-core = 聊天裡出現連結時，由你家的 Core 去取網頁的標題和圖片，網站看到的是你家伺服器的位址。

privacy-plugins-title = 第三方登入和插件網頁
privacy-plugins-oidc = 用別的帳號登入插件時，帳號和密碼只發給那個插件的伺服器，家裡的 Core 不保存第三方密碼。
privacy-plugins-web = 插件的網頁在你手機的瀏覽器核心裡打開，它能不能出網、出到哪，由插件自己決定。

privacy-device-title = 留在手機上的東西
privacy-device-files = 裝置金鑰、連線資訊、快取和日誌放在 App 的私有目錄裡，別的 App 讀不到；移除 App 時一起刪掉，伺服器上的歷史不受影響。
privacy-device-backup = 這些檔案會跟著系統備份走（iCloud 或 Google 備份），備份存在哪個帳號裡由系統的設定決定。

# License list load errors (crates/wonton-client/src/about/mod.rs).
licenses-parse-failed = 許可清單解析失敗：{ $error }
licenses-empty = 許可清單裡一個元件都沒有。
licenses-no-text = 許可 { $id } 沒有全文。
licenses-none-to-show = 許可清單裡沒有可顯示的許可。
