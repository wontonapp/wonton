## 檔案服務發給 App、Bot 和 Agent 的文字（ADR-0043）。一條完整的話一條訊息，不在程式碼裡拼；
## 資料（檔案編號、大小、外掛程式 ID、上游給的原因）作參數嵌進句子，不翻。
## 台灣用詞，引號「」，中英文之間加空格；對使用者叫「應用程式」，不叫「外掛程式」；不對使用者說「核心」，說「家用伺服器」。

files-not-found = 沒有這個檔案
files-moved-or-deleted = 檔案已被移動或刪除
files-not-shared = 這個檔案沒有分享給你
files-share-expired = 這個編號的分享過期了，再找一次檔案
files-changed = 檔案登記後改過，你看過的可能不是現在的內容，再找一次
files-share-not-authorized = 應用程式 { $plugin } 沒有共用資料夾 { $share } 的授權
files-mcp-gone = 應用程式 { $plugin } 在執行，但不再提供 MCP 服務，讀不了這個資源
files-plugin-unavailable = 來源應用程式 { $plugin } 解除安裝了或沒有在執行
files-notify-no-matrix = 家用伺服器沒有接入 Matrix（WONTON_MATRIX_URL），發不到「Wonton」私訊
files-matrix-send-failed = 發不到「Wonton」私訊：{ $error }
files-upstream-not-own = { $service } 不是應用程式 { $plugin } 自己的服務

# 預覽和縮圖。
files-too-big-for-thumbnail = 檔案超過 { $limit } MiB，不做縮圖
files-pdf-unreadable = 這個 PDF 解析不了（可能加密了或結構壞了），讀不了內容
files-pdf-no-text = 這個 PDF 沒有文字層（是掃描件或整頁圖），共 { $pages } 頁，看不到圖表裡的內容
files-too-big-for-parsing = 檔案超過 { $limit } MiB，抽不出內容
files-video-unsupported = 這個影片的封裝讀不了（只認 MP4、M4V 和 MOV），只能下載或分享
files-video-unreadable = 這個影片解析不了（封裝或結構不認識），拿不到時長和解析度
files-audio-unreadable = 音訊讀不了內容，只能下載或分享
files-thumbnail-timeout = 原檔案 { $after } 內沒有讀完
files-idle-limited = 應用程式的回應 { $idle } 沒有新的位元組
files-declared-size-over-limit = 應用程式聲明的大小超過 resources/read 的上限（{ $limit } MiB）
files-not-text-nul = { $name } 不是文字檔（內容裡有 NUL 位元組），讀不了內容：PDF 等二進位檔案這類只能下載或分享。
files-not-text-utf8 = { $name } 不是文字檔：內容解不成 UTF-8，讀不了內容。PDF 等二進位檔案這類只能下載或分享。
files-no-preview = 這個類型沒有預覽或縮圖
files-preview-failed = 做不了預覽或縮圖：{ $why }
files-decode-busy = 圖片解碼忙
files-view-busy = 這個檔案正在讀，稍後再試
files-read-busy = 圖片讀取忙

# 存檔案、上傳和刪除。
files-stage-no-space = 放不下這 { $total } 位元組，磁碟要留 5% 空閒
files-delete-transfers-only = 家用伺服器只刪共用資料夾 { $folder } 裡的檔案
files-size-mismatch = 傳完的資料有 { $actual } 位元組，建立時聲明的是 { $size } 位元組
files-no-hard-links = { $name }：共用資料夾 { $folder } 所在的檔案系統不支援硬連結
files-name-taken = { $preferred } 已經有 { $count } 個同名的檔案
files-short-read = 只讀到 { $copied } 位元組，應該有 { $size } 位元組
files-upload-disk-full = 磁碟放不下這個 { $size } 位元組的檔案（要留 { $percent }% 空閒，inode 也不能用完）
files-upload-leftover = 磁碟只剩 { $free } 位元組，這個上傳還差 { $left } 位元組
files-upload-quota-full = 磁碟配額滿了，這個上傳放不下
files-upload-inodes-full = 磁碟的 inode 用完了，這個上傳放不下
files-upload-record-label = 待完成的上傳記錄
files-upload-record-unwritable = 待完成的上傳記錄沒能寫入，先不放
files-path-collision = 這個路徑已經是別的檔案了：編號刪了，檔案沒動
files-residual-undeletable = 清不掉這個上傳的殘留，檔案沒有刪

# 讀應用程式資源用的名額。
files-plugin-read-overrun = 應用程式 { $plugin } 有讀取超過 { $seconds } 秒沒有結束
files-slots-full = 應用程式 { $plugin } 的讀取排滿了，稍後再試
files-plugin-slots-exhausted = 應用程式資源的讀取都佔滿了

# 從應用程式讀資源。
files-resource-invalid-url = 內部位址不合法：{ $error }
files-resource-http-only = 內部位址只能用 http
files-resource-no-credentials = 內部位址不能帶使用者名稱或密碼
files-resource-host = 內部位址的主機名稱要是應用程式的服務名稱，不用 IP 位址
files-resource-unreachable = 連不上應用程式的服務：{ $why }
files-resource-redirect = 應用程式的內部位址回了跳轉（{ $status }），家用伺服器不跟隨
files-upstream-status = 應用程式回傳 { $status }：{ $detail }
files-upstream-read-failed = 讀應用程式的回應失敗：{ $error }

# 刪檔案用的回收槽。
files-trash-foreign-slot = 回收槽 { $name } 不是家用伺服器建的（屬主 { $owner }、權限 { $mode }，應該是 { $euid }、700），要重啟家用伺服器才能再刪
files-trash-placeholder-missing = 回收槽 { $index } 的佔位檔案不見了，要重啟家用伺服器才能再刪
files-trash-full = { $folder } 裡的回收槽都被佔著，重啟家用伺服器找回裡面的檔案後才能再刪

# 共用資料夾。
files-check-share = 沒有共用資料夾 { $share }，用 wonton__list_shares 看有哪些

# 閘道的檔案介面。
files-service-disabled = 家用伺服器沒有啟用檔案服務
files-device-only = 檔案介面只給經隧道的已配對裝置，還要帶 { $header }: 1
files-stream-shortened = 檔案在下載途中變短了
files-invalid-room-body = 請求內容應為 {"{"}"room": …{"}"}：{ $error }

# 經 tusd 的上傳。
files-tusd-unconfigured = 家用伺服器沒有設定 tusd（WONTON_TUSD_URL）
files-tusd-required = 上傳介面要帶 Tus-Resumable: 1.0.0
files-tusd-timeout = tusd 遲遲沒有回應，稍後再試
files-tusd-gone = tusd 已經沒有這個上傳的資料了，要重傳
files-upload-disconnected = 建立這個上傳的請求已經斷開了
files-upload-length-required = 上傳要在建立時聲明長度（Upload-Length）
files-upload-filename-required = 上傳要在 Upload-Metadata 裡帶 filename
files-upload-finished = 這個上傳已經傳完了；要刪檔案用 DELETE /_wonton/files/{ $id }
files-upload-other-device = 這個上傳是另一部裝置建立的
files-upload-stale = 這個上傳很久沒有進度，已經清掉了，要重傳
files-hook-tusd-only = 檔案鉤子只給 tusd 用
files-hook-not-from-gateway = 這個請求沒有經過閘道
