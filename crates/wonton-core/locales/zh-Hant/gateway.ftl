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
gateway-bot-web-permission = 外掛程式網路裡的應用程式不能開啟 Bot 網頁或使用瀏覽器登入
gateway-bot-web-storage = 無法讀取 Bot 網頁記錄，請檢查伺服器儲存空間後重試

# 上游（身分提供者端點、模型閘道）出錯時顯示的名稱。
gateway-name-oidc = 身分提供者
gateway-name-models = 模型閘道

# TLS 入口容器後面的客戶端入口（ADR-0021）。
gateway-entry-header-invalid = 入口閘道沒有給這個請求標上正確的記號
gateway-entry-closed = 這個入口沒有開啟，主人可以在 Wonton 應用程式裡開啟
gateway-entry-public-denied = 公網位址上存取不了這個路徑
gateway-entry-setup-pending = 請先在這個應用程式的網頁上完成首次註冊，然後再試
gateway-entry-auth-unsupported = 這個應用程式還不支援用雲吞發的密碼登入
gateway-entry-state-unavailable = 讀不到這個應用程式的入口設定，請擁有者在雲吞裡檢查
gateway-entry-setup-closed = 應用程式 { $id } 裡已經沒有管理員帳號（資料可能被清空或還原過）。為防止有人搶註管理員，它的入口已自動關閉；確認應用程式就緒後在雲吞應用程式裡重新開啟。
gateway-entry-setup-timeout = 檢查應用程式首次註冊狀態逾時
gateway-entry-target-unavailable = 應用程式入口必須指向本應用程式網路中全應用程式共用的唯一服務

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
gateway-pair-role-unavailable = 這個邀請的角色無法使用，請管理員重新邀請。

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
# 静态网页的数据存储（ADR-0080 第 6 节）
gateway-web-storage-undeclared = 這個應用程式不儲存資料
gateway-web-storage-cross-site = 只有這個應用程式自己的頁面能修改它的資料
gateway-web-storage-not-ready = 替這個應用程式儲存資料的應用程式沒有在執行，請稍後再試

# 第三方客户端和公网访问（ADR-0021、ADR-0089）。
clients-header-missing = 請求少了 X-Wonton-Clients 標頭
clients-unauthenticated = 只有主人已配對的裝置能修改這些設定
clients-unavailable = 核心沒有跑在安裝好的容器裡，網域設定用不了
clients-domain-read-failed = 網域設定讀不了：{ $error }
gateway-chains-disabled = 家裡的伺服器沒有啟用呼叫鏈帳本

# agent_acp 的轉發（ADR-0061 第 5 塊）：建連時的拒絕和逐幀轉發裡的錯誤，按類別回給使用方。
gateway-agent-acp-websocket-only = 這個入口只收 WebSocket 連線
gateway-agent-acp-path = agent_acp 只有 /acp 一個入口
gateway-agent-acp-initiator-bot-required = 連線要用 X-Wonton-Bot 寫明發起調用的 Bot
gateway-agent-acp-initiator-bot-unrecognized = {$bot} 不是這個應用自己的 Bot
gateway-agent-acp-on-behalf-of = 連線要用 X-Wonton-On-Behalf-Of 寫明替誰說話
gateway-agent-acp-adopter-not-listened = {$bot} 不聽這個連線自報的說話人
gateway-agent-acp-target-not-listened = {$bot} 不聽這個人
gateway-agent-acp-no-bot = {$provider} 還沒有可用的 Bot，先把 Bot 建好
gateway-agent-acp-multi-bot = {$provider} 有幾個 Bot，先在詳情頁綁定要用哪個
gateway-agent-acp-token-missing = {$provider} 還沒有提供方權杖，重新安裝一次就有了
gateway-agent-acp-provider-unreachable = Agent 執行時沒有接起這條連線：{$detail}
gateway-agent-acp-busy = 這個應用開著的 Agent 連線已經到上限，稍後再試

gateway-install-rejected = 管理員沒有同意，沒有{$action}
gateway-install-expired = 管理員沒有在期限內回答，沒有{$action}
