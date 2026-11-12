## 商店、來源、網路設定、模型供應商和應用更新裡給 App 看的文字（ADR-0043）。
## 一句完整的話一則訊息，不在程式碼裡拼；資料（路徑、連接埠、映像檔名、ID、位址、Docker 的原話）
## 用參數放進句子，不翻。對使用者叫「應用程式」，不叫「外掛程式」；不說「核心」，說「家用伺服器」。

# ── 通用 ─────────────────────────────────────────────────────────────────────

## with_note 把原訊息和補充說明拼成一句。
plugin-settings-note-appended = { $message }（{ $note }）

# ── 網頁的區域網路連接埠的風險說明（lan/webui.rs，App 詳細頁顯示）────────────

plugin-settings-lan-webui-risk-impersonate = 區域網路裡的裝置，以及這部主機上能出網的應用程式，不用密碼就能以你的身分開啟它
plugin-settings-lan-webui-risk-register = 管理員註冊之前，區域網路裡的裝置和這部主機上能出網的應用程式可能搶先註冊
plugin-settings-lan-webui-risk-client = 區域網路裡的裝置拿到帳號和密碼就能連上它。在它自己的網頁裡改了密碼，這裡不會跟著變

# ── 資訊清單的 lan 和 lan.client（lan/mod.rs、lan/client.rs）─────────────────

plugin-settings-lan-client-prefix = lan.client：{ $message }
plugin-settings-lan-client-address-host = address 要包含 { $host }
plugin-settings-lan-client-template = 不能用 { $name }
plugin-settings-lan-need-ports = 申請了 network.lan 就必須用 lan.ports 寫明要開放哪些連接埠
plugin-settings-lan-need-permission = 宣告了 lan 就必須一併申請 network.lan 權限
plugin-settings-lan-no-matrix-bot = network.lan 不能和 matrix.bot 一起申請：位址變了要重建容器，家用伺服器不保存 Bot 的存取權杖
plugin-settings-lan-ports-empty = lan.ports 至少要一項
plugin-settings-lan-port-zero = lan.ports 的連接埠不能是 0
plugin-settings-lan-port-duplicate = lan.ports 裡的 { $port } 重複了
plugin-settings-lan-port-mcp = mcp 的連接埠只經家用伺服器存取，不能列進 lan.ports（{ $service } 的 { $port }）
plugin-settings-lan-network-none = lan.ports 裡的服務不能寫 network_mode: none

# ── lan.client 的連線資訊（lan/client.rs，詳細頁給使用者看）──────────────────

plugin-settings-lan-client-secret-missing = 密鑰庫裡沒有這個應用程式的記錄
plugin-settings-lan-client-container = 找不到容器 { $container }：{ $error }
plugin-settings-lan-client-not-published = 連接埠沒有發布在任何區域網路位址上

# ── 區域網路偵測和選擇（lan/detect.rs、lan/select.rs、lan/monitor.rs）────────

plugin-settings-lan-fixed-by-env = 區域網路位址由家用伺服器的 WONTON_LAN_ADDRESSES 指定，無法在設定裡變更
plugin-settings-lan-announce-by-env = 手機連的位址由家用伺服器的 WONTON_ANNOUNCE_ADDRESSES 指定，無法在設定裡變更
plugin-settings-lan-desktop-vm = 伺服器跑在電腦上的虛擬機裡，手機連不到虛擬機的位址。啟動器要把 WONTON_ANNOUNCE_ADDRESSES 設為電腦的區域網路位址。核心：{ $evidence }
plugin-settings-lan-announced = 宣告
plugin-settings-lan-fixed-missing = WONTON_LAN_ADDRESSES 裡的 { $ip } 不在主機上
plugin-settings-lan-detect-failed = 偵測主機的網路卡失敗：{ $failure }
plugin-settings-lan-detect-error = 偵測主機的網路卡時出錯：{ $error }
plugin-settings-lan-detect-core-image = 找不到家用伺服器容器 { $container } 的映像檔
plugin-settings-lan-detect-exit = 偵測容器以 { $code } 結束：{ $reason }
plugin-settings-lan-detect-output = 偵測容器的輸出不正確：{ $error }
plugin-settings-lan-detect-timeout = 容器 { $container } { $seconds } 秒內沒有跑完
plugin-settings-lan-detect-no-exit-code = Docker 沒有提供容器 { $container } 的結束代碼
plugin-settings-lan-listen-error = 檢查連接埠占用時出錯：{ $error }
plugin-settings-lan-choice-unknown = 已儲存的區域網路設定這個版本的家用伺服器看不懂，要重新選擇
plugin-settings-lan-no-default-route = 主機沒有 IPv4 預設路由
plugin-settings-lan-default-virtual = 預設路由走的是 { $name }（{ $why }），不自動選擇
plugin-settings-lan-default-no-private = 預設路由所在的網卡 { $name } 上沒有私有 IPv4 位址
plugin-settings-lan-interface-gone = 選的網卡 { $name } 已不在主機上，或上面沒有私有位址
plugin-settings-lan-interface-no-private = 選的網卡 { $name } 上沒有私有 IPv4 位址
plugin-settings-lan-address-gone = 選的位址 { $address } 已不在主機上
plugin-settings-lan-no-selection = 沒有選任何網卡或位址
plugin-settings-lan-choice-unrecognized = 不認得這種選法
plugin-settings-lan-interface-not-selectable = { $name } 不是主機上可以選的網卡
plugin-settings-lan-address-not-selectable = { $address } 不是主機上可以選的位址
plugin-settings-lan-kind-tunnel = 隧道網卡，如 Tailscale
plugin-settings-lan-kind-wireguard = WireGuard 網卡
plugin-settings-lan-kind-interface = 網卡
plugin-settings-lan-bridge-or-veth = Docker 的網橋或 veth
plugin-settings-lan-bound = { $interface } 的 { $ip }
plugin-settings-lan-select-failed = 選不出區域網路位址：{ $reason }
plugin-settings-lan-lookup-failed = 找不到區域網路連接埠發布在哪裡：{ $error }
plugin-settings-lan-rebuild-failed = 以新的區域網路位址重建失敗：{ $error }
plugin-settings-lan-check-failed = 核對應用程式失敗：{ $error }

# ── 安裝預覽裡的區域網路連接埠（lan/grant.rs）────────────────────────────────

plugin-settings-lan-port-line = 區域網路連接埠：{ $service } 的 { $key }，開在 { $place } 上
plugin-settings-lan-port-line-plain = 區域網路連接埠：{ $service } 的 { $key }
plugin-settings-lan-direct-notice = 區域網路裡的裝置直接連這些連接埠，不經家用伺服器登入，身分由應用程式自行驗證
plugin-settings-lan-conflict-taken = 區域網路連接埠 { $key } 已被應用程式 { $id } 佔用
plugin-settings-lan-note-reselect = 可以在區域網路設定裡改選網卡或位址

# ── 網頁的區域網路連接埠開關（lan/webui.rs）──────────────────────────────────

plugin-settings-lan-webui-no-switch = 應用程式 { $id } 沒有 lan_webui 開關：它的區域網路連接埠不用開關
plugin-settings-lan-webui-busy = 應用程式 { $id } 正在安裝、解除安裝或重建，等那邊做完
plugin-settings-lan-webui-close-failed = 關閉網頁的區域網路連接埠時失敗：{ $error }
plugin-settings-lan-webui-rollback-failed = 改開關失敗後的退回也失敗：{ $error }

# ── 安裝錯誤的整句（install_error.rs 各變體的構造點）────────────────────────

plugin-settings-not-installed = 應用程式 { $id } 沒有安裝
plugin-settings-permission-not-granted = 以下權限需要你確認後才能安裝：{ $permissions }
plugin-settings-core-upgrading = 家用伺服器正在更新，更新結束後再試
plugin-settings-bot-matrix-unavailable = 家用伺服器沒有連上 Matrix（WONTON_MATRIX_URL），宣告 matrix.bot 的應用程式裝不了
plugin-settings-note-revoke-device-failed = 註銷安裝裝置失敗：{ $error }
plugin-settings-note-migration-containers-gone = 遷移網路時舊容器已經刪除：重試這次重建，或重新安裝應用程式（可以不刪資料）復原
plugin-settings-note-registry-mirror = 可以在設定裡開啟下載鏡像站
plugin-settings-upgrade-rolled-back = 更新到 { $new } 失敗，已退回 { $version }
plugin-settings-upgrade-rollback-failed = 更新到 { $new } 失敗，退回 { $version } 也失敗：{ $error }
plugin-settings-upgrade-cleanup-unfinished = 收拾未完成，可能有殘留：{ $failures }
plugin-settings-upgrade-cleanup-model-credentials = 模型閘道憑證：{ $error }

# ── 出網代理（egress/）───────────────────────────────────────────────────────

plugin-settings-egress-plugin-and-url = 使用代理應用程式時不能同時填代理位址
plugin-settings-egress-restarts-changed = 要重新啟動的應用程式和確認時不一樣了：重新確認
plugin-settings-egress-cidr-invalid = 網段 { $cidr } 不正確：{ $error }
plugin-settings-egress-cidr-overlaps-network = 網段 { $cidr } 和 Docker 網路 { $network }（{ $subnet }）重疊：放行的話出口容器會直接撥到別的應用程式
plugin-settings-egress-env-locked = 代理位址由 WONTON_EGRESS_PROXY 指定，無法在這裡變更：改安裝目錄的設定後再執行一次安裝指令碼
plugin-settings-egress-address-invalid = 代理位址不正確：{ $error }
plugin-settings-egress-address-scheme = 不支援代理協定 { $scheme }：只能用 http、https 或 socks5
plugin-settings-egress-address-no-host = 代理位址裡沒有主機名稱
plugin-settings-egress-address-no-path = 代理位址只寫協定、主機和連接埠，不帶路徑和參數
plugin-settings-egress-address-utf8 = 代理位址裡的使用者名稱或密碼不是合法的 UTF-8：{ $error }
plugin-settings-egress-no-response = 代理 { $address } { $seconds } 秒沒有回應：主機防火牆要放行 Docker 網段到這個連接埠
plugin-settings-egress-refused = 代理 { $address } 拒絕連線：連接埠沒有人在聽，或者只監聽回環位址
plugin-settings-egress-unreachable = 連不上代理 { $address }：{ $error }
plugin-settings-egress-upstream-failed = 代理本身是通的，但它連不上外面：{ $error }
plugin-settings-egress-rejected = 代理這一步沒有通過：{ $error }
plugin-settings-egress-disconnected = 代理 { $address } 中斷了連線：{ $error }
plugin-settings-egress-not-http = 代理 { $address } 回的不是 HTTP
plugin-settings-egress-auth-407 = 代理 { $address } 回 407：要使用者名稱和密碼，或者寫錯了
plugin-settings-egress-status = 代理 { $address } 回 { $code }
plugin-settings-egress-relay-not-running = 代理容器沒有在執行
plugin-settings-egress-relay-detail = 直接連代理是通的，經代理容器不通：{ $detail }。查看容器 { $container } 的記錄檔
plugin-settings-egress-diagnose-exit = 查原因的容器以 { $code } 結束：{ $reason }
plugin-settings-egress-provider-missing = 沒有安裝能做出網代理的應用程式
plugin-settings-egress-provider-manifest = 讀不出代理應用程式 { $id } 的宣告
plugin-settings-egress-provider-conflict = 已經安裝了代理應用程式 { $id }：同一時間只能有一個
plugin-settings-egress-provider-not-running = 代理應用程式沒有在執行：{ $detail }
plugin-settings-egress-relay-cidr-overlap = 網段 { $cidr } 和代理容器接入的 Docker 網路重疊，沒有放行：放行的話它會直接撥到別的應用程式
plugin-settings-egress-token-unreadable = 服務 { $service } 把 Matrix 權杖寫在 command 或 configs 裡，讀不回來
plugin-settings-egress-token-reinstall = 重新安裝應用程式（可以不刪資料）後會簽發新裝置，Bot 要重新建立加密身分
plugin-settings-egress-token-read-back = Bot 的權杖讀不回來：{ $reason }。{ $reinstall }

plugin-settings-egress-bot-record-missing = 沒有這個應用程式的 Bot 記錄
plugin-settings-egress-old-container-gone = 舊容器 { $container } 已不在了
plugin-settings-egress-token-not-in-env = 舊容器 { $container } 的環境變數 { $variable } 裡沒有權杖
plugin-settings-egress-token-mismatch = 舊容器裡的權杖不一致
plugin-settings-egress-token-not-used = 舊版本沒有服務用到權杖

# ── 下載鏡像站（registry_mirror/）────────────────────────────────────────────

plugin-settings-mirror-address-invalid = 下載鏡像站位址 { $address } 不正確：只填主機名稱和可選連接埠，不帶協定、路徑或使用者名稱密碼
plugin-settings-mirror-policy-never = 映像檔 { $image } 標了 pull_policy: never，本機又沒有
plugin-settings-mirror-all-failed = 鏡像 { $image } 從哪個鏡像站位址都拉不下來；{ $tries }。請到設定裡測速、換一個鏡像站位址
plugin-settings-mirror-try-failed = { $mirror }：{ $error }
plugin-settings-mirror-digest-mismatch = 本機映像檔的 RepoDigests 沒有要求的摘要 { $digest }，拒絕使用
plugin-settings-mirror-probe-timeout = 測速總逾時（{ $ms } 毫秒）：{ $error }
plugin-settings-mirror-no-challenge = 缺少有效的 Bearer WWW-Authenticate
plugin-settings-mirror-token-request-rejected = 匿名權杖請求被拒絕
plugin-settings-mirror-token-response-invalid = 匿名權杖回應無效：{ $error }
plugin-settings-mirror-token-missing = 匿名權杖回應缺少 token/access_token
plugin-settings-mirror-manifest-rejected = 清單請求被拒絕
plugin-settings-mirror-manifest-digest = 清單摘要不符：期望 { $digest }，收到 { $actual }
plugin-settings-mirror-status = HTTP { $status }：{ $detail }

# ── 應用商店和來源（storefront/、store.rs、store_info.rs）────────────────────

plugin-settings-store-source-unreadable = 新增的來源讀不了：{ $why }
plugin-settings-store-source-not-added = 沒有新增過 { $address }
plugin-settings-store-source-limit = 最多新增 { $max } 個來源，先刪掉不用的
plugin-settings-store-source-save-failed = 新增的來源存不了：{ $error }
plugin-settings-store-source-in-official = { $repo } 已經在官方商店裡了
plugin-settings-store-source-neither = { $repo } 的根目錄沒有 { $compose }，也沒有 { $index }：不是應用程式，也不是索引
plugin-settings-store-source-already-added = { $address } 已經新增過了
plugin-settings-store-source-index-missing = { $provider } 沒有索引檔案
plugin-settings-store-source-not-an-index = { $address } 不是索引：{ $error }
plugin-settings-store-source-plugin-missing = 索引 { $provider } 裡沒有 { $id }
plugin-settings-store-community-partial = 有的沒讀到：{ $missing }
plugin-settings-store-community-page-failed = 搜尋結果的第 { $page } 頁（{ $error }）
plugin-settings-store-community-unavailable = 取不到社群應用程式：{ $error }
plugin-settings-store-system-app = 系統應用程式只能隨家用伺服器預先安裝
plugin-settings-store-permission-unsupported = 家用伺服器還不支援權限 { $permission }
plugin-settings-store-index-newer = 索引的格式是 { $format }，這個版本的家用伺服器只看得懂 { $supported }：更新家用伺服器後才能用新的索引
plugin-settings-store-index-invalid = 無法讀取索引：{ $error }
plugin-settings-store-index-compose-digest = compose 和 files 裡 { $file } 的 sha256 對不上
plugin-settings-store-index-version-mismatch = version { $version } 和資訊清單的版本 { $manifest } 不一致
plugin-settings-store-index-source-duplicate = source 重複
plugin-settings-store-index-id-duplicate = 應用程式 ID { $id } 重複
plugin-settings-store-index-source-with-version = source 不能帶版本
plugin-settings-store-index-commit-length = commit 要是 40 位提交號
plugin-settings-store-index-file-digest = { $repo } 的 { $path } 和索引裡記的 sha256 對不上
plugin-settings-store-index-file-missing = 索引裡 { $source } 的檔案沒有 { $path }
plugin-settings-store-index-blob-invalid-sha = 官方索引裡的 sha256 不正確：{ $sha256 }
plugin-settings-store-index-blob-missing = 索引目錄裡沒有 { $path }：{ $error }
plugin-settings-store-record-id-mismatch = 記錄裡的 ID 是 { $record }，與檔名 { $file } 不符

# ── 商店資訊清單的資料（store_info.rs，安裝官方應用程式時給 App 的原因）──────

plugin-settings-store-info-spec-missing = 資訊清單要寫 spec，如 spec: "{ $version }"
plugin-settings-store-info-spec-string = spec 必須是字串，如 "{ $version }"（YAML 裡要加引號，不然會讀成數字）
plugin-settings-store-info-spec-version = spec { $text } 不是規範版本（如 "{ $version }"）
plugin-settings-store-info-author = author.name 不能為空，不超過 { $max } 個字元
plugin-settings-store-info-license = license { $license } 不是 SPDX 授權條款表達式（如 MIT、AGPL-3.0-only）：{ $error }
plugin-settings-store-info-screenshots-count = screenshots 要有 1 到 { $max } 張
plugin-settings-store-info-screenshot-format = 截圖 { $raw } 要是 PNG 或 JPEG
plugin-settings-store-info-screenshot-duplicate = 截圖 { $raw } 重複了
plugin-settings-store-info-tags-count = tags 最多 { $max } 個
plugin-settings-store-info-tag-length = 標籤 { $tag } 要有 1 到 { $max } 個字元，前後不留空白
plugin-settings-store-info-tag-control = 標籤 { $tag } 裡有控制字元
plugin-settings-store-info-tag-duplicate = 標籤 { $tag } 重複了
plugin-settings-store-info-field-length = { $field }.{ $lang } 超過 { $max } 個字元
plugin-settings-store-info-https-url = { $field } { $value } 要是 https:// 開頭的網址

# ── 更新檢查（upgrade/）────────────────────────────────────────────────────

plugin-settings-upgrade-none = 應用程式 { $id } 沒有新版本
plugin-settings-upgrade-no-origin = 應用程式 { $id } 沒有來源，查不到新版本
plugin-settings-upgrade-id-changed = 應用程式的套件是 { $package }，要更新的是 { $record }：更新不能改 id
plugin-settings-upgrade-same-version = 和裝著的 { $version } 一樣，不用換
plugin-settings-upgrade-changed = { $id } 的新版本在預覽之後變了（套件摘要現在是 { $digest }），要重新預覽、重新確認
plugin-settings-upgrade-older = 比裝著的 { $version } 舊。應用程式的資料可能已經按新版本遷移過，舊版本可能讀不了，先備份
plugin-settings-upgrade-source-changed = 來源從 { $old } 換成 { $new }，換了之後雲吞不再按原來的來源提示更新
plugin-settings-upgrade-origin-upload = 你上傳的檔案
plugin-settings-upgrade-origin-index = { $via } 的索引
plugin-settings-upgrade-index-unavailable = 官方索引沒取到，查不了新版本：{ $details }
plugin-settings-upgrade-system-app = 系統應用程式隨家用伺服器更新，這裡無法更新
plugin-settings-upgrade-lan-ports-closing = 區域網路連接埠 { $ports } 更新後會關掉，要繼續開著就打開下面的開關
plugin-settings-upgrade-check-failed = { $id } 的新版本沒查到：{ $error }
plugin-settings-upgrade-official-commit-moved = 官方索引裡版本 { $version } 換了提交：不算新版本，裝著的沒變
plugin-settings-upgrade-tag-moved = 這個版本的 tag { $tag } 被作者改到別的提交：不算新版本，裝著的沒變
plugin-settings-upgrade-index-removed = 索引 { $provider } 已經從商店刪掉，不再提示新版本
plugin-settings-upgrade-compose-not-utf8 = { $file } 不是 UTF-8 文字
plugin-settings-upgrade-manifest-id-changed = 資訊清單的 id 變成了 { $id }
plugin-settings-upgrade-manifest-version = 資訊清單的版本是 { $version }
plugin-settings-upgrade-bundled-failed = 隨家用伺服器帶來的 { $version } 沒有更新成功，還在跑 { $from }，下次啟動再試：{ $error }

# ── 存取 GitHub（github.rs、github_source.rs）───────────────────────────────

plugin-settings-github-tags-what = 讀 { $repo } 的 tag
plugin-settings-github-no-repo = GitHub 上沒有公開儲存庫 { $repo }
plugin-settings-github-status = { $what } 時 GitHub 回了 { $status }
plugin-settings-github-tags-parse = 無法解析 GitHub 回傳的 { $repo } tag：{ $error }
plugin-settings-github-file-what = 取得 { $repo } 在提交 { $commit } 裡的 { $path }
plugin-settings-github-missing = 沒有 { $what }
plugin-settings-github-url-what = 取得 { $url }
plugin-settings-github-api-what = 經 API 取 { $repo } 在 { $reference } 裡的 { $path }
plugin-settings-github-tree-what = 讀取 { $repo } 在 { $commit } 中 { $dir } 的檔案清單
plugin-settings-github-rate-limited = { $what }：GitHub 限速了，幾分鐘後再試
plugin-settings-github-rate-limited-plain = GitHub 限速了，幾分鐘後再試
plugin-settings-github-api-status = { $what }：GitHub 回了 { $status }
plugin-settings-github-search-url = 搜尋的位址不對：{ $error }
plugin-settings-github-search-what = 搜尋社群應用程式
plugin-settings-github-search-status = 搜尋社群應用程式時 GitHub 回了 { $status }
plugin-settings-github-search-parse = 無法解析 GitHub 回傳的搜尋結果：{ $error }
plugin-settings-github-context = { $what }：{ $error }
plugin-settings-github-timeout = { $what }：請求逾時，沒有在時限內答完
plugin-settings-github-unreachable = { $what }：連不上 GitHub（{ $error }）
plugin-settings-github-too-big = 超過 { $max } KiB
plugin-settings-github-read-what = 讀回應
plugin-settings-github-no-tag = { $repo } 沒有 tag { $tag }
plugin-settings-github-no-release = { $repo } 沒有正式版本：要有 v<語意化版本> 的 tag，如 v1.0.0
plugin-settings-github-no-release-tag = 沒有 v<語意化版本> 的正式版本 tag
plugin-settings-github-tag-version-mismatch = tag { $tag } 和資訊清單的版本 { $version } 不一致：發布時 tag 要是 v<資訊清單的版本>
plugin-settings-github-tag-version-short = tag { $tag } 和資訊清單的版本 { $version } 不一致
plugin-settings-github-source-invalid = 來源 `{ $text }` 不對：{ $why }；寫成 github:<owner>/<repo>[/<子目錄>]，可以帶 @<tag> 或 @<40 位提交號>
plugin-settings-github-source-scheme = 只支援 github: 開頭的來源
plugin-settings-github-source-no-name = 缺少儲存庫名稱
plugin-settings-github-source-name-chars = owner 和儲存庫名稱只能有字母、數字、`-`、`_`、`.`
plugin-settings-github-source-subdir-empty = 子目錄裡有空的路徑段；開頭或結尾的 `/` 也算空段
plugin-settings-github-source-subdir-long = 子目錄超過 200 個字元
plugin-settings-github-source-subdir-dot = 子目錄裡有 `.` 或 `..` 段
plugin-settings-github-source-subdir-chars = 子目錄只能有字母、數字、`-`、`_`、`.`、`/`
plugin-settings-github-source-bad-tag = @ 後面的 tag 不合法
plugin-settings-github-sibling-url = { $url } 旁邊的 { $path } 不是合法位址：{ $error }

# ── 來源和索引的位址（source_address.rs、index_location.rs）─────────────────

plugin-settings-source-address-invalid = 位址 `{ $text }` 不對：{ $why }
plugin-settings-source-address-tls = 只支援 https：未審核的來源要靠 TLS 確認內容沒被改過
plugin-settings-source-address-https-only = 只支援 https 位址和 GitHub 儲存庫
plugin-settings-source-address-formats = 寫成儲存庫位址、owner/repo，或索引的 https 位址
plugin-settings-source-not-a-repo = `{ $text }` 不是應用程式儲存庫。索引要在商店裡新增，再從那裡安裝
plugin-settings-source-page-invalid = GitHub 位址 `{ $path }` 不對：寫成 https://github.com/<owner>/<repo>，可以帶 /tree/<tag>[/<子目錄>] 或 /commit/<提交>
plugin-settings-index-address-invalid = 索引位址 `{ $text }` 不對：{ $why }
plugin-settings-index-address-https = 只支援 https
plugin-settings-item-source-invalid = 來源 `{ $source }` 不對

# ── 模型閘道和模型供應商（models.rs、model_providers.rs）────────────────────

plugin-settings-models-gateway-not-running = 模型閘道 { $id } 沒在執行
plugin-settings-models-gateway-absent = 沒有安裝模型閘道（宣告 provides.models 的系統應用程式）
plugin-settings-models-revoke-not-running = 模型閘道沒在執行，吊銷不了 { $id } 的憑證
plugin-settings-models-revoke-provider-not-running = 模型閘道 { $provider } 沒在執行，吊銷不了 { $id } 的憑證
plugin-settings-models-reconcile-failed = 模型閘道憑證對帳失敗：{ $error }
plugin-settings-models-credential-rejected = 模型閘道 { $provider } 不認家用伺服器簽發的憑證 { $name }：簽發後請求模型清單仍然 401
plugin-settings-models-unreachable = 模型閘道 { $provider } 連不上：{ $error }
plugin-settings-models-list-error = 模型閘道 { $provider } 回答模型清單時出錯：HTTP { $status }
plugin-settings-models-provider-error = 預設模型的服務商出錯：{ $error }
plugin-settings-models-provider-invalid-reply = 服務商沒有回傳合法的對話結果
plugin-settings-models-list-malformed = 模型閘道 { $provider } 的模型清單格式不合法
plugin-settings-models-connect-first = 先在「模型設定」接上模型，Bot 才能回覆。
plugin-settings-models-not-ready = 模型閘道 { $provider } 未就緒：{ $error }
plugin-settings-models-command-rejected = 模型閘道 { $provider } 拒絕了憑證命令（{ $command }，結束代碼 { $code }）：{ $stderr }
plugin-settings-model-action-save = 儲存
plugin-settings-model-action-test = 測試
plugin-settings-model-disconnected-saved = 供應商連不上，key 已儲存
plugin-settings-model-disconnected-unsaved = 供應商連不上，什麼都沒儲存
plugin-settings-model-no-provider-commands = 模型閘道 { $provider } 沒有宣告供應商命令
plugin-settings-model-provider-unsupported = 模型閘道不支援供應商 { $id }
plugin-settings-model-custom-unsupported = 模型閘道不支援自訂供應商
plugin-settings-model-custom-encode-failed = 自訂供應商的請求編碼失敗
plugin-settings-model-command-timeout = 模型閘道 { $provider } 命令執行失敗或逾時
plugin-settings-model-invalid-endpoint = 供應商位址或模型名稱不合法
plugin-settings-model-key-rejected = 供應商拒絕了 key，key 沒有儲存
plugin-settings-model-provider-command-failed = 模型閘道 { $provider } { $action }供應商失敗，結束代碼 { $code }
plugin-settings-model-list-failed = 模型閘道 { $provider } 列出供應商失敗：{ $error }
plugin-settings-model-list-exit = 模型閘道 { $provider } 列出供應商失敗，結束代碼 { $code }
plugin-settings-model-list-not-json = 模型閘道 { $provider } 的供應商清單不是合法 JSON
plugin-settings-model-key-invalid = key 不能空白、不能有換行，且不能超過 4 KiB
plugin-settings-model-url-invalid = 供應商位址必須是 http 或 https URL、不能有空白，且不能超過 2 KiB
plugin-settings-model-name-invalid = 模型名稱不能空白、不能有空白或控制字元，且不能超過 200 位元組
plugin-settings-model-request-format = 模型供應商的請求格式不正確

# ── 商店索引（storefront）──────────────────────────────────────────────────

plugin-settings-store-index-fetch-failed = 取不到官方索引：{ $error }

# ── 健康檢查（health.rs）───────────────────────────────────────────────────

plugin-settings-health-timeout = 容器 { $container } 在期限內沒有就緒（最後狀態：{ $state }）
plugin-settings-health-unhealthy = 容器 { $container } 健康檢查失敗
plugin-settings-health-exited = 容器 { $container } 已結束（結束代碼 { $code }）
plugin-settings-health-no-healthcheck = 容器 { $container } 被要求 service_healthy，但沒有設定健康檢查
plugin-settings-health-webui-timeout = 網頁入口 { $url } 在期限內沒有回應：{ $last_error }
plugin-settings-health-log-tail = 最近日誌：
plugin-settings-health-probes = 最近探針：
plugin-settings-health-probe-line = [結束代碼 { $code }] { $output }
plugin-settings-health-probe-line-unknown = [結束代碼 ?] { $output }
plugin-settings-health-container-missing = 查不到容器 { $container }
plugin-settings-health-one-shot-exit = 一次性服務的容器 { $container } 沒有以 0 結束（{ $status }，結束代碼 { $exit_code }）
plugin-settings-health-not-running = 容器 { $container } 不在執行（{ $status }）

# ── 應用程式密鑰（secrets.rs）──────────────────────────────────────────────────

plugin-settings-secret-missing = 應用程式密鑰 { $name } 遺失
plugin-settings-secret-not-text = 應用程式密鑰 { $name } 不是文字
plugin-settings-secret-invalid = 應用程式密鑰 { $name }：容器 { $container } 裡的 { $path } { $reason }
plugin-settings-secret-not-regular = 不是一般檔案
plugin-settings-secret-too-big = 超過 64 KiB

# ── 應用程式的 Bot（bots.rs）───────────────────────────────────────────────────

plugin-settings-bot-avatar-missing = bot.avatar 指向的 { $path } 不在應用程式的套件裡
plugin-settings-bot-avatar-type = bot.avatar 只支援 png、jpg、gif、webp：{ $path }
plugin-settings-bot-device-name = Wonton 應用程式 { $id }（{ $bot }）
plugin-settings-bot-account-deactivated = Bot 帳號 { $user } 已停用：選擇刪除資料解除安裝這個應用程式後重裝，會分配新帳號
plugin-settings-bot-device-record-missing = 沒有 Bot { $bot } 的裝置記錄：重裝應用程式（可以不刪資料）
plugin-settings-bot-device-revoked = Bot { $user } 的裝置已被吊銷，應用程式拿著的 Matrix 權杖失效了：重裝應用程式（可以不刪資料）重新簽發

# ── 區網偵測、出口容器和商店索引的補充（lan/detect.rs、docker_limits.rs、egress、storefront）──

plugin-settings-lan-detect-wait-exit = 等一次性容器結束
plugin-settings-lan-detect-read-output = 讀一次性容器的輸出
plugin-settings-lan-detect-list-networks = 列出網路
plugin-settings-limits-update-container = 更新容器 { $name }
plugin-settings-mirror-docker-proxy = 查詢 Docker 的代理設定
plugin-settings-mirror-inspect = 檢查映像檔
plugin-settings-mirror-inspect-image = 檢查映像檔 { $image }
plugin-settings-mirror-pull-image = 拉取映像檔 { $image }
plugin-settings-egress-rebuild-failed = 按新的代理設定重建失敗：{ $error }
plugin-settings-egress-gost-config = 產生 gost 的設定失敗：{ $error }
plugin-settings-egress-prepare-dir = 準備 { $dir } 失敗：{ $error }
plugin-settings-egress-write-file = 寫 { $path } 失敗：{ $error }
plugin-settings-egress-remove-dir = 刪除 { $dir } 失敗：{ $error }
plugin-settings-store-index-no-format = 沒有 format
plugin-settings-store-index-docs-problem = 無法讀取官方索引的 docs：{ $error }
plugin-settings-store-index-onboarding-problem = 無法讀取官方索引的 onboarding：{ $error }
plugin-settings-store-index-mirrors-problem = 無法讀取官方索引的 registry_mirrors：{ $error }
plugin-settings-store-index-read-failed = 讀不了官方索引 { $path }：{ $error }

plugin-secret-unknown = 請求裡有不認識的密鑰：{ $names }
plugin-secret-required-missing = 必填的密鑰還沒填：{ $names }
plugin-secret-required-clear = 必填的密鑰 { $name } 不能清空
plugin-secret-not-input = { $name } 不是主人填寫的密鑰
chat-card-secrets-not-used = 這張確認不收密鑰

# ── 插件設定的值和 Schema（ADR-0077）────────────────────────────────────────

plugin-settings-schema-invalid-json = 設定檔不是合法 JSON
plugin-settings-schema-object = 設定 Schema 必須是物件 Schema
plugin-settings-schema-properties = 設定 Schema 缺少 properties 物件
plugin-settings-schema-too-many = 設定 Schema 的項目超過 64 個
plugin-settings-schema-draft = $schema 必須是 https://json-schema.org/draft/2020-12/schema
plugin-settings-schema-additional = additionalProperties 必須是 false
plugin-settings-schema-unsupported = 原生控制項不支援 { $key }
plugin-settings-schema-required-duplicate = required 裡的 { $key } 重複了
plugin-settings-schema-required-unknown = required 裡的 { $key } 不是屬性
plugin-settings-schema-required = required 必須是屬性名稱清單
plugin-settings-schema-order = x-wonton-order 裡每個屬性最多出現一次
plugin-settings-schema-field = 每個屬性必須是 Schema 物件
plugin-settings-schema-name = 屬性名稱以小寫字母開頭，只用小寫字母、數字和 _，最長 64 個字元
plugin-settings-schema-secret-name = 這個名稱也是金鑰名稱，改掉其中一個
plugin-settings-schema-sensitive = 敏感項目，移到輸入金鑰
plugin-settings-schema-title = 缺少 x-wonton-title
plugin-settings-schema-description = x-wonton-description 必須按語言鍵寫
plugin-settings-schema-type = type 必須是單一受支援的類型
plugin-settings-schema-enum = enum 至少要有一個不重複的字串
plugin-settings-schema-pattern = pattern 不是合法正規表示式
plugin-settings-schema-bound = minimum 和 maximum 必須是數字
plugin-settings-schema-length = minLength 和 maxLength 必須是非負整數
plugin-settings-schema-items = 陣列的 items 必須是字串 enum
plugin-settings-schema-unique = 陣列項目必須寫 uniqueItems: true
plugin-settings-schema-enum-duplicate = enum 裡有重複的值
plugin-settings-schema-enum-titles = x-wonton-enum-titles 必須給每個列舉值一個標題，不多不少
plugin-settings-schema-problems = 設定 Schema 有 { $count } 處不合格
plugin-settings-value-unknown-field = 這個應用程式沒有這一項
plugin-settings-value-missing = 必填，填一下
plugin-settings-value-wrong-type = 必須是 { $type }
plugin-settings-value-below-minimum = 不能小於 { $min }
plugin-settings-value-above-maximum = 不能大於 { $max }
plugin-settings-value-too-short = 至少 { $min } 個字元
plugin-settings-value-too-long = 最多 { $max } 個字元
plugin-settings-value-pattern = 不符合要求的格式
plugin-settings-value-duplicate = 有重複的值
plugin-settings-value-not-in-options = 從選項裡挑一個
plugin-settings-value-string-too-big = 超過 { $max } 位元組
plugin-settings-value-array-too-big = 超過 { $max } 項
plugin-settings-value-too-big = 設定合計超過 { $max } 位元組
plugin-settings-value-unknown-type = 這個 App 版本送不出這種值，更新 App
plugin-settings-value-problems = { $count } 項設定不合格
