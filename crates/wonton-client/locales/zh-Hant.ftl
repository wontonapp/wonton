## 預設收合的技術詳細資訊。原因、錯誤碼和上游原文保持不變。
## 不限長度。參數是診斷資料，不是翻譯鍵。

# CoreError 前綴。$reason 是原始錯誤詳細資訊。
core-error-storage = 無法讀寫本機儲存空間：{ $reason }
core-error-not-ready = 尚未就緒：{ $reason }
core-error-transport = 無法連上家用伺服器：{ $reason }
core-error-unauthorized = 家用伺服器拒絕了請求：{ $reason }
core-error-upstream = 家用伺服器發生錯誤：{ $reason }

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

# 其他伺服器（ADR-0050）。預設收起的「詳情」。
# $reason 是原始錯誤詳情。
peers-transport = 連不上家裡的伺服器：{ $reason }
# $seconds 是請求逾時的秒數。
peers-timeout = { $seconds } 秒內沒有回應
# 還沒有連線資訊或本機代理，請求送不出去。
peers-not-connected = 還沒連上家用伺服器。
# 殼把不是伺服器配對的操作發到了這裡。
peers-invalid-action = 不是伺服器配對操作。

# $reason 是本機代理位址或 HTTP 用戶端的原始錯誤。
peers-invalid-proxy = 本機代理不可用：{ $reason }
# $status 是原始 HTTP 狀態碼。
peers-server-status = 家裡的伺服器回了 HTTP { $status }
# $reason 是 JSON 解析器的原始錯誤。
peers-invalid-response = 家裡的伺服器給的回應讀不懂：{ $reason }
# $reason 是隧道拒絕的原話。
peers-rejected = 家裡的伺服器拒絕了這台裝置：{ $reason }

# 初始設定的步驟操作：落在錯的步驟上、狀態沒取到時的原因。
onboarding-steps-not-in-flow = 不在初始設定裡
onboarding-steps-already-committed = 已經點了完成
onboarding-steps-not-profile-step = 不在填寫資料這一步
onboarding-steps-not-agent-step = 不在選 Agent 這一步
onboarding-steps-state-missing = 還沒有取到初始設定狀態
onboarding-steps-agent-not-listed = 這個 Agent 不在可選清單裡
onboarding-steps-agent-not-installable = 這個 Agent 無法安裝
onboarding-steps-not-apps-step = 不在選應用程式這一步
onboarding-steps-need-not-listed = 這個項目不在推薦清單裡
onboarding-steps-committing = 正在送出
onboarding-steps-no-installable-agent = 沒有選能安裝的 Agent
onboarding-steps-testing = 正在測試
onboarding-steps-model-not-chosen = 還沒有選模型
onboarding-steps-cannot-skip = 這一步不能跳過
onboarding-steps-cannot-go-back = 這一步不能返回

# 接模型這一步。
onboarding-model-key-required = API 金鑰不能為空
onboarding-model-not-model-step = 還沒到接模型這一步
onboarding-model-testing = 已經在測試
onboarding-model-providers-missing = 還沒取到供應商清單
onboarding-model-provider-not-listed = 這個供應商不在清單裡
onboarding-model-custom-unsupported = 模型閘道不支援自訂供應商
onboarding-model-name-required = 模型名稱不能為空
onboarding-model-base-url-invalid = Base URL 要是 https://… 這樣的位址
onboarding-model-step-or-testing = 不在接模型這一步，或正在測試
onboarding-model-none-configured = 模型閘道裡還沒有接好的模型
onboarding-model-state-missing = 還沒有取到初始設定狀態
onboarding-model-login-missing = 選中的 Agent 沒有這種登入方式

# 最後一頁。被攔下的項目帶著這則說明，直到預覽重新整理。
onboarding-review-permissions-changed = 要的權限和確認時的不一樣了，到應用程式頁面重新安裝
# $installed 和 $listed 是安裝來源提供的和清單裡的 ID，不翻譯。
onboarding-review-source-mismatch = 安裝來源裡的應用程式是 { $installed }，不是清單裡的 { $listed }
onboarding-review-items-not-ready = 還有沒準備好的項目
onboarding-review-item-missing = 最後一頁上沒有這一項
onboarding-review-no-failure = 這一項沒有失敗

## chat-a 片段：crates/wonton-client/src/chat/{login,sign_in,recovery}.rs。
## 是聊天問題/錯誤預設收合的「詳細資訊」：CoreError 的原因和 ChatProblem 的 message。

# 登入相關。$reason 是 Matrix SDK 的原始錯誤詳細資訊。
chat-sign-in-restore-failed = 無法恢復 Matrix 工作階段：{ $reason }
chat-sign-in-no-connection-info = 還沒有家用伺服器的連線資訊
chat-sign-in-login-failed = 無法登入 Matrix：{ $reason }
chat-sign-in-no-saved-session = 登入後取不到登入狀態

# 取得登入權杖的請求。$reason 是上游原文，或翻譯後的逾時詳細資訊。
# $reason 是代理位址剖析器的原始錯誤。
chat-login-invalid-proxy = 本機代理伺服器位址無效：{ $reason }
# $reason 是 HTTP 用戶端的原始錯誤。
chat-login-http-client = 無法建立 HTTP 用戶端：{ $reason }
# $seconds 是請求逾時的秒數。
chat-login-timeout = { $seconds } 秒內未收到回應
chat-login-request-failed = 無法向家用伺服器取得 Matrix 登入權杖：{ $reason }
# $reason 是讀取 HTTP 回應的原始錯誤。
chat-login-response-failed = 無法讀取家用伺服器的回應：{ $reason }
# $reason 是 JSON 剖析器的原始錯誤。
chat-login-invalid-token = 家用伺服器傳回的登入權杖格式有誤：{ $reason }
# $login_type 是家用伺服器給的登入方式；$expected 是要求的登入方式。
chat-login-wrong-type = 家用伺服器給的登入方式 { $login_type } 不是 { $expected }
# $status 是原始 HTTP 狀態碼。
chat-login-server-status = 家用伺服器傳回 HTTP { $status }
# $message 和 $kind 是伺服器的原始錯誤說明和錯誤碼。
chat-login-server-error = { $message }（{ $kind }）

# 恢復相關。
chat-recovery-not-connected = 還沒連上家用伺服器
chat-recovery-reset-no-response = 家用伺服器沒有回應，備份沒有動，稍後再試
chat-recovery-empty-key = 復原金鑰不能為空
# $reason 是儲存的原始錯誤詳細資訊。
chat-recovery-key-not-saved = 復原金鑰無法儲存到本機，請現在就抄下來：{ $reason }
# 回答或停止時段沒成功時，確認卡片下方問題詳細資訊裡顯示的話。
# $message 是家用伺服器的原始錯誤說明，或翻譯後的詳細資訊。
chat-confirmation-answer-failed = 回答沒有成功：{ $message }
# $error 是 HTTP 用戶端的原始錯誤。
chat-confirmation-answer-lost = 沒等到家用伺服器的回應，不確定是否已答上，可以再點一次（{ $error }）
# $message 是家用伺服器的原始錯誤說明，或翻譯後的詳細資訊。
chat-confirmation-stop-failed = 沒有停下來：{ $message }
# $error 是 HTTP 用戶端的原始錯誤。
chat-confirmation-stop-lost = 沒等到家用伺服器的回應，可以再點一次（{ $error }）
# $error 是 JSON 剖析器的原始錯誤。
chat-confirmation-response-unreadable = 讀不懂家用伺服器的回應（{ $error }）
# $status 是原始 HTTP 狀態碼。
chat-confirmation-core-status = 家用伺服器傳回 HTTP { $status }
# 開始私聊時使用者名稱為空時顯示。
chat-contacts-empty-username = 使用者名稱不能為空。
# $reason 是 Matrix ID 剖析器的原始錯誤。
chat-contacts-invalid-user-id = 使用者 ID 無效：{ $reason }

# 透過 Siri 或捷徑傳送訊息、內容為空時顯示。
chat-ask-empty-message = 訊息不能為空。
# Siri 或捷徑傳入的 Bot ID 剖析失敗時顯示。
chat-ask-invalid-bot-id = 預設 Agent 的 ID 格式不對。
# 已有私訊的聊天室 ID 剖析失敗時顯示。
chat-ask-invalid-room-id = 私訊的聊天室 ID 無效。

# Matrix 撤銷本裝置、帳號標記為失敗時顯示。
chat-session-device-revoked = 這台裝置在 Matrix 上已被撤銷，需要主人重新配對。
## 檔案傳輸與播放的詳細資訊。顯示在傳輸清單項目、檔案卡片和播放失敗說明上。

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
files-playback-transport = 連不上家用伺服器。可以下載後用其他 App 開啟。
# 這部裝置無法播放這個格式時顯示。
files-playback-unsupported = 這部手機無法播放這個格式。可以下載後用其他 App 開啟。
# $detail 是家用伺服器或連線的原始原因。
files-playback-unavailable = 家用伺服器無法提供這個檔案：{ $detail }
# $detail 是家用伺服器或連線的原始原因。
files-playback-error = 家用伺服器發生錯誤：{ $detail }。可以下載後用其他 App 開啟。
# 播放探測逾時時顯示。
files-playback-no-response = 家用伺服器一直沒有回應

# $reason 是原始 io 錯誤。
files-stream-local-address = 無法開啟播放用的本機位址：{ $reason }
# 隧道未連線時播放失敗的診斷詳細資訊。
files-stream-not-connected = 隧道尚未連上
# $code 是隧道原始錯誤碼。
files-stream-tunnel-unavailable = 隧道無法使用（{ $code }）
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
home-pairing-bind-failed = 無法建立網路連線：{ $error }
# 底欄（tabs/view.rs、tabs/config.rs）和外觀（appearance/mod.rs）的介面「詳細資訊」。
# 對使用者，插件一律叫「App」；底欄按 Android 慣例寫「底部導覽列」。

# 底欄 ViewModel 的錯誤，作為 CoreError 的 reason 或底欄的問題提示顯示。
# 無參數。
tabs-view-always-pinned = 聊天和應用程式已經固定到底部導覽列。
tabs-view-plugin-list-not-ready = 還沒載入應用程式清單，暫時無法操作應用程式頁面。
tabs-view-unknown-plugin = 找不到這個應用程式。
tabs-view-plugin-no-page = 這個應用程式沒有可開啟的網頁。

# 底部導覽列設定檔（tabs.json）的錯誤，作為 CoreError 的 reason 顯示。
# $path 是設定檔的路徑，$reason 是原始的剖析或讀寫錯誤。
tabs-config-read-failed = 無法讀取底部導覽列設定檔 { $path }：{ $reason }
tabs-config-corrupt = 底部導覽列設定檔 { $path } 已損毀：{ $reason }
# $count 是底部導覽列最多能放的項目數（MAX_ITEMS）。
tabs-config-too-many-items = 底部導覽列最多只能有 { $count } 個項目。
tabs-config-duplicate-page = 底部導覽列裡有重複的頁面。
tabs-config-plugin-page-incomplete = 應用程式頁面缺少標題和圖示。
tabs-config-missing-fixed = 底部導覽列缺少聊天或應用程式。
# $path 是設定檔的路徑，$reason 是原始的寫入錯誤。
tabs-config-write-failed = 無法寫入底部導覽列設定檔 { $path }：{ $reason }

# 外觀設定的錯誤。appearance-settings-unreadable 作為外觀頁的問題提示（AppearanceProblem.message）顯示，其餘是它裡面的 reason 或 CoreError::Storage 的 reason。
# $path 是設定檔的路徑，$reason 是原始的剖析或讀寫錯誤。
appearance-settings-unreadable = 無法讀取外觀設定，先使用預設值：{ $reason }
appearance-settings-read-failed = 無法讀取外觀設定檔 { $path }：{ $reason }
appearance-settings-corrupt = 外觀設定檔 { $path } 已損毀：{ $reason }
# $value 是設定檔裡的原始顏色字串，原樣不翻。
appearance-settings-invalid-accent = 外觀設定檔 { $path } 裡的顏色無效：{ $value }
# $path 是設定檔的路徑，$reason 是原始的寫入錯誤。
appearance-settings-write-failed = 無法寫入外觀設定檔 { $path }：{ $reason }

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

# 多台伺服器（crates/wonton-client/src/servers.rs、client/push_route.rs）。
servers-removed = 這台伺服器已經從這支手機上移除。
servers-push-from-removed = 傳來這則訊息的伺服器已經從這支手機上移除。
servers-push-scope-closed = 那台伺服器已經關閉了。
# $seconds 是等待登記的秒數。
servers-push-register-timeout = { $seconds } 秒內未能完成推播通知註冊。

chat-avatar-cache-not-file = 頭像快取路徑不是檔案
chat-avatar-cache-hash-collision = 頭像快取的雜湊值衝突
chat-avatar-cache-missing-mxc = 頭像快取缺少 MXC 位址
chat-avatar-cache-path-not-utf8 = 頭像快取路徑不是有效的 UTF-8
scope-task-crashed = 背景工作異常結束：{ $error }
scope-switched-server = 已經切換到另一台伺服器
