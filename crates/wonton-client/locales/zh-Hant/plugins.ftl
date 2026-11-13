# 傳輸或播放還沒開始就發現未連線時顯示。
files-api-not-connected = 還沒連上家用伺服器

# $reason 是代理位址剖析器的原始錯誤。
files-api-invalid-proxy = 本機代理伺服器位址無效：{ $reason }

# $reason 是 HTTP 用戶端的原始錯誤。
files-api-http-client = 無法建立 HTTP 用戶端：{ $reason }

# $reason 是 JSON 剖析器的原始錯誤。
files-api-invalid-response = 無法解析家用伺服器的回應：{ $reason }

# 家用伺服器不再認得這部裝置時顯示。
files-api-unauthenticated = 家用伺服器不再認得這部裝置

# $code 是隧道原始錯誤碼。
files-api-tunnel-unavailable = 隧道無法使用（{ $code }）

# $status 是原始 HTTP 狀態碼。
files-api-status = 家用伺服器傳回 HTTP { $status }

# 原檔案和選擇時不一樣時顯示。
files-upload-source-changed = 原檔案改過了，要重新選擇

# 家用伺服器對進度查詢（HEAD）的回應缺少 Upload-Offset 標頭時顯示。
files-upload-head-no-offset = 家用伺服器對進度查詢的回應沒有 Upload-Offset

# 殼層無法開啟這類來源時顯示。
files-upload-no-opener = 殼層沒有設定 FileOpener，無法開啟這個來源

# 來源不是一般檔案、無法定位時顯示。
files-upload-source-not-file = 這個來源不是可定位的一般檔案，無法續傳

# $reason 是開啟原檔案的工作的異常資訊。
files-upload-open-task-failed = 開啟原檔案的工作異常結束：{ $reason }

# $reason 是原始 io 錯誤。
files-upload-source-denied = 系統不允許讀取原檔案：{ $reason }

# $reason 是原始 io 錯誤。
files-upload-source-missing = 原檔案不見了：{ $reason }

# $reason 是原始 io 錯誤。
files-upload-source-read = 無法讀取原檔案：{ $reason }

# 家用伺服器建立上傳的回應缺少合法的 Location 標頭時顯示。
files-upload-create-no-location = 家用伺服器建立上傳的回應沒有合法的 Location

# 家用伺服器對進度查詢（HEAD）的回應缺少 Upload-Length 標頭時顯示。
files-upload-head-no-length = 家用伺服器對進度查詢的回應沒有 Upload-Length

# 超過停滯時限沒有位元組在動時顯示。
files-upload-stalled = 上傳沒有進展，隧道可能卡住了

# 家用伺服器一直不回應最後一個 PATCH 時顯示。
files-upload-no-response = 家用伺服器一直沒有回應

# 原檔案比記錄的短時顯示。
files-upload-source-shrank = 原檔案變短了

# $reason 是原始 io 錯誤。
files-download-write = 無法寫入下載的檔案：{ $reason }

# 家用伺服器一直不回應下載請求時顯示。
files-download-no-response = 家用伺服器一直沒有回應

# 半截檔案比家用伺服器那邊的檔案還長時顯示。
files-download-shrunk = 檔案變短了，從頭下載

# 續傳的範圍和已下載的對不上時顯示。
files-download-range-mismatch = 家用伺服器給的範圍對不上，從頭下載

# 連線中斷、位元組沒有收完時顯示。
files-download-incomplete = 連線中斷，沒有收完

# 超過停滯時限收不到資料時顯示。
files-download-stalled = 收不到資料，隧道可能卡住了

# 連線中斷導致播放失敗時顯示。
files-playback-transport = 連不上家用伺服器。

# 這部裝置無法播放這個格式時顯示。
files-playback-unsupported = 這部手機無法播放這個格式。可以下載後用其他 App 開啟。

# $detail 是家用伺服器或連線的原始原因。
files-playback-unavailable = 家用伺服器無法提供這個檔案：{ $detail }

# $detail 是家用伺服器或連線的原始原因。
files-playback-error = 家用伺服器發生錯誤：{ $detail }。

# 播放探測逾時時顯示。
files-playback-no-response = 家用伺服器一直沒有回應

# $reason 是原始 io 錯誤。
files-stream-local-address = 無法開啟播放用的本機位址：{ $reason }

# 隧道未連線時播放失敗的診斷詳細資訊。
files-stream-not-connected = 隧道尚未連上

# $code 是隧道原始錯誤碼。
files-stream-tunnel-unavailable = 隧道無法使用（{ $code }）

# 底欄 ViewModel 的錯誤，作為 CoreError 的 reason 或底欄的問題提示顯示。
# 無參數。
tabs-view-always-pinned = 聊天和應用程式已經固定到底部導覽列。

tabs-view-plugin-list-not-ready = 還沒載入應用程式清單，暫時無法操作應用程式頁面。

tabs-view-unknown-plugin = 找不到這個應用程式。

# $error 是本地儲存的原始錯誤。
tabs-storage-problem = 我的應用存不下，下次啟動可能恢復舊狀態：{ $error }

tabs-view-plugin-no-page = 這個應用程式沒有可開啟的網頁。

tabs-page-not-placed = 這個頁面不在面板裡。

# 底部導覽列設定檔（tabs.json）的錯誤，作為 CoreError 的 reason 顯示。
# $path 是設定檔的路徑，$reason 是原始的剖析或讀寫錯誤。
tabs-config-read-failed = 無法讀取我的應用程式設定檔 { $path }：{ $reason }

tabs-config-corrupt = 我的應用程式設定檔 { $path } 已損毀：{ $reason }

# $count 是底部導覽列最多能放的項目數（MAX_ITEMS）。
tabs-config-too-many-items = 底部導覽列最多只能有 { $count } 個項目。

tabs-config-duplicate-page = 我的應用程式裡有重複的頁面。

tabs-config-plugin-page-incomplete = 應用程式頁面缺少標題和圖示。

tabs-config-missing-fixed = 底部導覽列缺少聊天。

# $path 是設定檔的路徑，$reason 是原始的寫入錯誤。
tabs-config-write-failed = 無法寫入我的應用程式設定檔 { $path }：{ $reason }

scope-task-crashed = 背景工作異常結束：{ $error }

scope-switched-server = 已經切換到另一台伺服器

## 用戶端核心剩下的部分（ADR-0043）：C1、C2 沒涵蓋的模組的「詳細資訊」，預設收合。同一批檔案裡的記錄檔寫英文，不在這裡。
## 推播通知裡給人看的內文和佔位（未知的傳送者、邀請你加入、[檔案]……）還沒定，不在這次範圍。
# 檔案傳輸（crates/wonton-client/src/files/mod.rs）：被拒絕的操作、失敗的操作和本機檔案清理的詳細資訊。
files-invalid-action = 不是檔案操作

files-name-required = 檔案名稱不能為空

files-download-already-running = 這個檔案已經在下載了

files-transfer-not-found = 找不到這筆傳輸

# 這筆傳輸不在「等著傳送連結」的狀態時要重新傳送連結。
files-transfer-server-changed = 換了家用伺服器，這筆傳輸屬於之前那一台

files-dismiss-transfer-running = 這筆傳輸還在進行，要先取消

# 移除上傳並要求刪除本機檔案時：原檔案是系統給的，App 刪不了它。
files-dismiss-upload-source-protected = 上傳的原始檔案不由雲吞管理

# $reason 是原始的 io 錯誤。
files-delete-local-failed = 無法刪除手機上的檔案：{ $reason }

# 背景檔案操作失敗的結果。$id 是原樣的檔案編號，$message 是原始的失敗詳細資訊。
files-delete-failed = 無法刪除檔案 { $id }：{ $message }

files-send-failed = 無法把檔案 { $id } 傳到裝置：{ $message }

files-grant-failed = 無法授權檔案 { $id }：{ $message }

files-play-failed = 無法播放檔案 { $id }：{ $message }

# $path 是傳輸清單的路徑，$reason 是原始的讀寫錯誤。
files-store-write-failed = 無法寫入傳輸清單 { $path }：{ $reason }

# 工作階段清單（crates/wonton-client/src/session_store.rs）。
sessions-title-required = 工作階段標題不能為空

# $id 是原樣的工作階段編號。
sessions-not-found = 沒有這個工作階段：{ $id }

# 殼把不認得的操作交給工作階段時顯示。
sessions-invalid-action = 不是工作階段操作

# 應用程式頁面的橋接（crates/wonton-client/src/tabs/bridge/）。
tabs-bridge-page-not-open = 這個 App 的頁面沒有開啟

tabs-bridge-invalid-environment = 頁面欄高與安全區必須是有限的非負數

tabs-bridge-public-page = 公網網頁沒有 Wonton 的橋。

tabs-page-cannot-reload = 這個頁面不能重新載入。

tabs-bar-not-pinned = 這個頁面沒有固定到底欄。

tabs-bar-already-pinned = 這個頁面已經在我的應用程式裡。

tabs-bar-replace-required = 底欄已滿，必須指定要替換的頁面。

tabs-bar-replacing-missing = 要替換的頁面不在底欄。

tabs-bar-fixed = 聊天和應用不能被替換。

tabs-bar-not-full = 底欄未滿時不能指定替換頁面。

tabs-bar-remove-fixed = 聊天和應用不能拿掉。

tabs-bar-page-missing = 這個頁面不在我的應用程式。

tabs-bar-index-out-of-range = 位置超出我的應用程式的範圍。

tabs-open-only-plugins = 只有外掛頁面可以設定開啟方式。

tabs-fullscreen-only-plugins = 只有外掛頁面可以全螢幕。

tabs-fullscreen-none = 目前沒有全螢幕頁面。

tabs-fullscreen-not-current = 退出的頁面不是目前全螢幕頁面。
tabs-view-not-a-page = 只有應用程式頁面能在這裡開啟。
tabs-view-no-page-open = 現在沒有開啟著的應用程式頁面。
