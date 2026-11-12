## 預設收合的技術詳細資訊。原因、錯誤碼和上游原文保持不變。
## 不限長度。參數是診斷資料，不是翻譯鍵。

# CoreError 前綴。$reason 是原始錯誤詳細資訊。
core-error-storage = 無法讀寫本機儲存空間：{ $reason }
core-error-not-ready = 尚未就緒：{ $reason }
core-error-transport = 無法連上家用伺服器：{ $reason }
core-error-unauthorized = 家用伺服器拒絕了請求：{ $reason }
core-error-forbidden = 家用伺服器不允許這個操作：{ $reason }
core-error-upstream = 家用伺服器發生錯誤：{ $reason }
core-error-server-too-old = 家裡的伺服器太舊，還做不了這件事。升級伺服器後再試。

## 外掛程式模組的技術詳細資訊，預設收合在錯誤、應用程式任務和代理伺服器、下載鏡像站設定頁的「詳細資訊」裡。
## 不限長度。參數是診斷資料，不是翻譯鍵。
# 外掛程式管理介面（crates/wonton-client/src/plugins/api.rs）。
# $reason 是原始錯誤詳細資訊。
plugins-api-invalid-proxy = 本機代理伺服器位址無效：{ $reason }

plugins-api-http-client = 無法建立 HTTP 用戶端：{ $reason }

# $reason 是 JSON 剖析器的原始錯誤。
plugins-api-invalid-response = 家用伺服器傳回的內容格式有誤：{ $reason }

# $reason 是 JSON 序列化器的原始錯誤。
plugins-api-invalid-request = 無法寫出請求主體：{ $reason }

# $reason 是被拒絕的圖片位址，或 URL 剖析器的原始錯誤。
plugins-api-not-image-url = 不是應用程式圖片的位址：{ $reason }

# $status 是原始 HTTP 狀態碼。
plugins-api-server-status = 家用伺服器傳回 HTTP { $status }

plugins-api-not-connected = 連上家用伺服器後再試。

plugins-api-server-too-old = 家裡的伺服器 { $version } 太舊：這個功能要 { $minimum } 起。

# 安裝、更新、解除安裝任務（crates/wonton-client/src/plugins/tasks.rs）。
plugins-tasks-uncertain = 連線中斷了，等不到家用伺服器的結果，看一下已安裝清單。

plugins-tasks-server-changed = 換了家用伺服器，等不到原來那台的結果。

plugins-tasks-preview-no-source = 這份預覽沒有來源，裝不了。

# $id 是原樣的任務 ID。
plugins-tasks-no-such-task = 沒有這個任務：{ $id }

plugins-tasks-task-not-done = 任務還沒做完。

# $title 是應用程式標題，按商店裡的原樣顯示。
plugins-tasks-task-running = { $title } 正在安裝、更新或解除安裝。

# 代理伺服器設定（crates/wonton-client/src/plugins/egress.rs）。
plugins-egress-save-in-progress = 上一次變更還沒做完。

plugins-egress-not-loaded = 還沒取到代理伺服器的設定。

plugins-egress-no-confirmation = 沒有等待確認的變更。

plugins-egress-env-locked = 代理由 WONTON_EGRESS_PROXY 指定，不能在這裡改。

plugins-egress-plugin-and-address = 使用代理應用程式時不能同時填寫代理位址。

plugins-egress-plugin-not-installed = 還沒有安裝代理應用程式。

# $reason 是連線問題的原始詳細資訊。
plugins-egress-save-uncertain = 不確定改好了沒有，連上後再看一下：{ $reason }

# 下載鏡像站設定（crates/wonton-client/src/plugins/registry_mirror.rs）。
plugins-registry-save-in-progress = 上一次下載鏡像站的變更還沒做完。

plugins-registry-not-loaded = 還沒取到下載鏡像站的設定。

# $reason 是連線問題的原始詳細資訊。
plugins-registry-save-uncertain = 不確定改好了沒有，連上後重新讀取設定：{ $reason }

matrix-login-not-current = 這次登入已經結束，或者已經換成了新的一次。

matrix-login-wrong-step = 這次登入不在這一步。

matrix-login-method-not-offered = 這台伺服器不提供這種登入方式。

matrix-login-device-name-required = 請填這台裝置的名稱。

matrix-login-invalid-user-id = 請填完整的 Matrix ID，例如 @name:matrix.org。

matrix-login-password-required = 請填密碼。

matrix-login-redirect-not-allowed = App 收不到發往這個位址的登入回呼。

# 啟動家裡的應用程式（crates/wonton-client/src/plugins/start.rs）。
# $id 是原樣的應用程式編號。
plugins-start-busy = { $id } 正在啟動、安裝、更新或解除安裝

# 家用伺服器的一鍵更新（crates/wonton-client/src/plugins/core_upgrade.rs）：不確定階段的說明和被拒絕的操作，顯示在更新頁和錯誤的「詳細資訊」裡。
# 無參數。家用伺服器給了這個 App 不認得的結果。
plugins-core-upgrade-unrecognized = 家用伺服器回傳了這個版本的雲吞看不懂的結果，更新雲吞後再查看

# 代替缺少的執行中摘要。不是一句話。
plugins-core-upgrade-unknown-digest = 未知

# 家用伺服器回報更新完成，但正在執行的和目標對不上。$running 是執行中的摘要或「未知」，$target 是原樣的目標摘要。
plugins-core-upgrade-completed-mismatch = 家用伺服器說更新完成了，但正在執行的不是目標版本：執行中的是 { $running }，目標是 { $target }

# 超過時限沒連上，結果未知。$digest 是退回舊版本要固定的原樣摘要。
plugins-core-upgrade-timed-out = 超過時限仍連不上家用伺服器，不知道更新的結果。連上後這裡會更新

plugins-core-upgrade-timed-out-rollback = 超過時限仍連不上家用伺服器，不知道更新的結果。連上後這裡會更新。要退回舊版本，在主機上再執行一次安裝指令，加上 WONTON_CORE_PIN={ $digest }

plugins-core-upgrade-invalid-action = 不是更新伺服器的操作

plugins-core-upgrade-already-running = 已經在更新了

plugins-core-upgrade-not-cancellable = 現在無法取消

# $reason 是連線問題的原始詳細資訊。
plugins-core-upgrade-start-uncertain = 不確定發起了沒有，連上後再看一下：{ $reason }

# 相容性檢查（ADR-0085）的詳情：每個原因一行，{ $raw } 是清單裡的原值。用戶端核心按
# App 的語言寫，升級頁列在新版本讀不了的應用程式下面。
plugins-core-upgrade-compat-spec = 它按規範 { $raw } 寫成，新版本不支援：升級後會被標為異常。

plugins-core-upgrade-compat-feature = 它的清單用了新版本不認得的 { $raw }：升級後會被標為異常。

plugins-core-upgrade-compat-capability = 它依賴能力 { $raw }，新版本不提供：升級後會被標為異常。

plugins-core-upgrade-compat-other = 新版本讀不了這個應用程式：升級後會被標為異常。

plugins-core-upgrade-compat-other-raw = 新版本讀不了這個應用程式的 { $raw }：升級後會被標為異常。

## 要跨來源隔離的插件網頁在系統瀏覽器裡開啟（ADR-0033）。後三條顯示在瀏覽器裡。
browser-front-failed = 無法開啟給瀏覽器用的本機位址：{ $error }

browser-page-not-isolated = 這個頁面不在瀏覽器裡開啟。

browser-not-paired = 雲吞還沒和家裡的伺服器配對。

browser-unknown-address = 這個位址已經失效。回到雲吞，重新開啟這個應用程式。

browser-not-connected = 雲吞目前沒有連上家裡的伺服器。回到雲吞，等連上後再重新整理這個頁面。

browser-bad-request = 請從雲吞裡開啟這個頁面。

# App 鎖的設定（crates/wonton-client/src/app_lock.rs，ADR-0102）。
# $path 是設定檔的路徑，$reason 是原始的 IO 錯誤。
app-lock-write-failed = 無法寫入 App 鎖設定檔 { $path }：{ $reason }
app-lock-read-failed = 無法讀取 App 鎖設定檔 { $path }：{ $reason }
app-lock-corrupt = App 鎖設定檔 { $path } 已損壞：{ $reason }
