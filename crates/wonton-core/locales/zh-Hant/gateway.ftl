## 閘道和家用伺服器自己出的網頁發給瀏覽器、App 和 Bot 的文字（ADR-0043）。一條完整的話一條訊息，
## 不在程式碼裡拼；資料（主機名稱、編號、狀態、上游給的原因）作參數嵌進句子，不翻。
## 台灣用詞，引號「」，中英文之間加空格；對使用者叫「應用程式」，不叫「外掛程式」；不對使用者說「核心」，說「家用伺服器」。

# 閘道介面、MCP 匯流排和工具策略共用。
gateway-mcp-disabled = 家用伺服器沒有啟用 MCP 匯流排

# 登入頁。
gateway-login-title = 登入
gateway-login-heading = 登入 Wonton
gateway-login-token-hint = 用權杖登入（家用伺服器資料目錄下的 <code>core/owner-token</code>）；手機上的雲吞配對後改為掃碼登入。
gateway-login-failed = 權杖不正確。
gateway-login-submit = 登入

# 狀態頁。
gateway-status-title = 狀態
gateway-status-logged-in = 已登入：{ $user }
gateway-status-apps-heading = 應用程式
gateway-status-empty = 還沒有安裝應用程式。
gateway-status-logout = 登出

# 狀態頁上應用程式的狀態。
gateway-state-installing = 安裝中
gateway-state-running = 執行中
gateway-state-stopped = 已停止
gateway-state-degraded = 異常：{ $reason }
gateway-state-uninstalling = 解除安裝中
gateway-state-updating = 更新中

# 錯誤頁。
gateway-page-error-title = 出錯了
gateway-page-error-detail = 詳情
gateway-not-found = 沒有這個頁面
gateway-cross-site-write = 家用伺服器只接受自己頁面發出的修改請求

# 按 Host 分流和外掛程式子網域上的錯誤。
gateway-unknown-host = 未知的位址 { $host }
gateway-oidc-unconfigured = 家用伺服器沒有設定身分提供者
gateway-no-app-for-host = 沒有安裝對應 { $host } 的應用程式
gateway-no-webui = 這個應用程式沒有網頁入口
gateway-app-not-ready = 應用程式 { $id } 尚未就緒
gateway-plugin-not-shared = 應用程式 { $id } 沒有開放給家人
gateway-proxy-sso-failed = 拿不到應用程式 { $id } 的代登入憑證：{ $error }
gateway-cross-site-login = 閘道不認其他網頁發出的跨站請求的登入狀態，請從應用程式自己的頁面操作
gateway-login-required = 需要先登入 Wonton

# 上游（身分提供者端點、模型閘道）出錯時顯示的名稱。
gateway-name-oidc = 身分提供者
gateway-name-models = 模型閘道

# SSO 交接和 OIDC 登入、授權確認。
gateway-sso-invalid-next = 跳轉位址不合法
gateway-sso-local-only = 只能跳轉到本機的外掛程式位址
gateway-sso-missing-code = 缺少交接碼
gateway-sso-code-invalid = 交接碼無效或已過期，請重新開啟
gateway-sso-user-mismatch = 身分提供者記住的是另一個使用者
gateway-proxy-sso-member-missing = 你的帳號已經不在家裡伺服器上，要主人重新邀請
gateway-sso-consent-mismatch = 授權請求與目前登入的使用者不一致

# 轉發給上游。
gateway-upstream-unreachable = { $name } 暫時無法連線：{ $error }
gateway-upstream-timeout = { $name } 遲遲沒有回應，可以稍後重試
gateway-invalid-path = 請求路徑不合法

# 外掛程式管理介面。
gateway-invalid-body = 請求內容不正確：{ $error }
gateway-invalid-package = 不是外掛程式套件：要 tar、tar.gz、zip 或 compose 檔案
gateway-denied = 這部裝置沒有配對或已被移除，要重新配對；命令列要帶 Authorization: Bearer <權杖>
gateway-cross-site-api = 應用程式管理介面不接受其他網頁發出的請求
gateway-disk-space = 無法查詢磁碟空間：{ $error }
gateway-invalid-lan-choice = 請求內容不是區域網路設定的選項：{ $error }
gateway-icon-not-found = 沒有這張圖片
gateway-icon-failed = 無法處理圖示：{ $error }

# 裝置和配對碼。
gateway-devices-not-found = 沒有登記這部裝置
gateway-devices-invalid-id = 請求內容應為 {"{"}"endpoint_id": "<64 位十六進位>"{"}"}
gateway-devices-invalid-addrs = 請求內容應為 {"{"}"direct_addrs": ["IP:連接埠"]{"}"}：{ $error }
gateway-devices-already-paired = 伺服器上已經有配對的裝置，不再發首次配對碼
gateway-devices-no-addresses = 隧道還沒連上中繼，請求裡也沒給直接連線位址，碼裡沒有能連的位址

# 模型閘道。
gateway-models-permission = 應用程式 { $id } 沒有申請 models 權限
gateway-plugin-network-only = 這個位址只給外掛程式網路裡的應用程式用
gateway-models-down = 模型閘道沒有在執行

# 初始設定。
gateway-onboarding-invalid-step = step 只能是 profile 或 finished，不是 { $step }
gateway-onboarding-invalid-agent = primary_agent 不是可選的 Agent：{ $agent }
gateway-onboarding-index-missing = 官方索引沒有 onboarding 清單

# 一鍵更新。
gateway-upgrade-disabled = 家用伺服器沒有啟用一鍵更新

# pusher 註冊。
gateway-pushers-invalid-body = 無法讀取 pusher 註冊的請求內容：{ $error }
gateway-pushers-invalid-json = pusher 註冊的請求內容不是 JSON：{ $error }
gateway-pushers-proxy-only = pusher 只能指向家裡的推送代理 { $url }
gateway-pushers-kind-refused = 不放行 { $kind } 類的 pusher
gateway-pushers-missing-kind = pusher 註冊的請求內容缺少 kind

# MCP 匯流排和工具策略（依主人的語言，Agent 和 Bot 會讀取）。
gateway-tools-permission = 應用程式 { $id } 沒有申請 tools 權限
gateway-agent-post-only = 這個介面只收 POST
gateway-body-unreadable = 無法讀取請求內容：{ $error }
gateway-body-invalid = 請求內容不合法：{ $error }
gateway-bot-not-found = 沒有這個 Bot：{ $caller }
gateway-bot-not-in-plugin = 應用程式 { $id } 沒有這個 Bot：{ $caller }
gateway-tool-not-found = 應用程式 { $id } 沒有工具 { $tool }
gateway-tool-deletes = 工具 { $tool } 會刪除或覆寫資料，每次都要確認，不能標成唯讀
gateway-plugin-not-running = 這個應用程式沒有在執行，或者沒有宣告工具

# App 回答確認請求。
gateway-confirm-invalid-body = 請求內容要是 {"{"}"answer": "approve" | "approve_wider" | "allow_window" | "reject"{"}"}：{ $error }
gateway-no-allow-window = 沒有生效中的放行時段

# 配對（發給掃碼的那支手機；依主人的語言）。
gateway-pair-unknown-code = 伺服器沒有發過這個配對碼
gateway-pair-code-used = 這個配對碼已經用過了，請換一個新碼
gateway-pair-code-expired = 配對碼過期了，請換一個新碼
gateway-pair-first-used = 這部伺服器已經有配對的裝置，首次配對碼不能再用；請在伺服器上執行 sudo wonton pair 另發一個配對碼
gateway-pair-register-failed = 伺服器登記裝置失敗：{ $error }

# 主人在 App 裡查看應用程式的金鑰。
gateway-owner-secrets-plugin-not-installed = 應用程式 { $id } 沒有安裝
gateway-owner-secrets-record-unreadable = 應用程式 { $id } 的記錄讀不出來，列不出它的金鑰
gateway-owner-secrets-not-viewable = 應用程式 { $id } 沒有宣告可查看的金鑰 { $name }
gateway-owner-secrets-not-generated = 金鑰 { $name } 還沒有值：啟動一次應用程式（或升級）會重新產生
gateway-owner-secrets-store-failed = 讀應用程式金鑰失敗：{ $error }
gateway-owner-secrets-not-text = 金鑰 { $name } 存的值不是文字
gateway-owner-secrets-audit-unavailable = 家用伺服器沒有接審計日誌，不能查看金鑰的值
gateway-plugin-scope-unsupported = 這個核心不支援修改外掛給哪些 Bot 用，請先升級核心。

# MCP Apps 宿主页（#35）。
mcp-app-loading = 正在載入…
mcp-app-failed = 介面沒能載入：{ $error }
mcp-app-page-failed = 介面沒能載入。
mcp-app-retry = 重試

# 好友經分享連線開啟網頁（ADR-0051）。
gateway-share-identity = 這台伺服器認不出你是誰
gateway-share-required = 主人沒有把這個應用程式分享給你
gateway-share-revoked = 主人停止分享這個應用了
gateway-share-expired = 分享已到期
gateway-confirmation-gone = 沒有這張確認，或它已結束超過一天
