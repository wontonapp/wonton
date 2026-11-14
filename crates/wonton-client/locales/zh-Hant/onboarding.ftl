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

# $seconds 是綁定的時限秒數。建立連線時本機網路在這段時間內沒準備好，會自動再試。
home-bind-timeout = 本機網路在 { $seconds } 秒內沒有準備好。

# $reason 是連線資訊剖析器的原始錯誤。
home-ticket-corrupt = 儲存的連線資訊已損毀：{ $reason }

home-no-connection-info = 還沒有家用伺服器的連線資訊

# 本機連線初始化失敗；底層錯誤放在獨立診斷中。
home-proxy-start-failed = App 無法啟動本機連線代理，請重試。

home-tunnel-closed = 隧道用戶端已關閉

home-lan-unsupported = 這個版本的用戶端核心不支援區域網路設定

home-port-mapping-save-failed = 連接埠映射的開關存不下來：{ $error }

# 連線的本機儲存空間（crates/wonton-client/src/home/storage.rs）。
# $path 是檔案的路徑，$reason 是原始的剖析或讀寫錯誤。
home-storage-device-key-corrupt = 裝置金鑰檔案 { $path } 已損毀：{ $reason }

home-storage-io-failed = 無法讀寫 { $path }：{ $reason }

# 配對監看時限：$seconds 是整次配對的監看時限，不是交換階段的時限。
home-pairing-timeout = 配對未能在 { $seconds } 秒內完成。

home-pairing-aborted = 配對沒有完成就中斷了，再試一次。

home-pairing-not-pairing = 沒有正在進行的配對。

home-pairing-setup-failed = 裝置已配對，但無法設定連線。請開啟診斷資訊查看原因。

# 可讀配對詳細資訊。$reason 已本地化；elapsed 是實際總耗時，stage 是每個交換階段的時限，單位均為秒。
home-pairing-failed-detail = { $reason }本次總耗時 { $elapsed_seconds } 秒，配對交換的每個階段時限為 { $stage_seconds } 秒。

home-pairing-joining-pending = 這是加入用的碼，正在等管理員同意。
home-pairing-joining-pending-role = 這是加入用的碼，正在等管理員同意。同意後加入的角色是「{ $role }」。

# 失敗類別不嵌入底層錯誤原文；原始資訊保留在診斷中。
home-failure-transport = 無法建立與家用伺服器的連線，或連線已中斷。請檢查手機和伺服器的網路後重試。
home-failure-unauthorized = 家用伺服器拒絕了這台裝置，請從已授權的裝置或伺服器重新配對。
home-failure-upstream = 已連上家用伺服器，但它需要的服務尚未就緒，請稍後重試。
home-failure-protocol = App 與家用伺服器無法使用相同的協定連線，請檢查兩端是否需要更新。
home-network-setup-failed = App 無法準備好手機的網路連線，會自動重試。
home-pairing-network-setup-failed = App 無法準備好配對所需的手機網路連線，請重試。
home-proxy-route-failed = App 無法登記本機連線路由，請重試。

# 僅在使用者主動開啟的可複製診斷中顯示；配對金鑰和中繼權杖會遮去。
home-tunnel-diagnostic = 錯誤碼：{ $code }。原始錯誤：{ $error }
home-diagnostic-unavailable = 無法安全讀取儲存的連線資訊，因此無法顯示診斷資訊。
home-pairing-diagnostic = 直連位址：{ $addresses }。中繼：[{ $relays }]。{ $error }

# 配對碼驗證，不包含掃描內容或金鑰。
home-code-not-pairing = 這不是雲吞配對碼，請掃描家用伺服器提供的碼。
home-code-unsupported-version = 無法辨識此配對碼的格式，請更新 App 後重試。
home-code-invalid-version = 配對碼缺少格式版本，請取得新碼後重試。
home-code-invalid-address = 配對碼中的伺服器連線資訊缺少或無效，請取得新碼後重試。
home-code-invalid-relay = 配對碼中的中繼資訊無效。中繼位址須使用 HTTPS，請取得新碼後重試。
home-code-invalid-domain = 配對碼中的伺服器位址無效，請取得新碼後重試。
home-code-invalid-secret = 配對碼中的配對金鑰缺少或無效，請取得新碼後重試。
home-code-invalid = 配對碼中的連線資訊無效，請取得新碼後重試。
home-code-rejected = 伺服器沒有接受此配對碼，請取得新碼後重試。
home-code-expired = 此配對碼已過期，請取得新碼後重試。
home-code-used = 此配對碼已被使用或撤銷，請取得新碼後重試。
home-code-closed = 伺服器不再接受這種配對碼，請從已授權的裝置或伺服器新增裝置。
home-code-request-invalid = 伺服器無法處理配對請求，請檢查 App 和伺服器是否需要更新。
home-code-server-error = 伺服器無法完成配對，請稍後重試。
