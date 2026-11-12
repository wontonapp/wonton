## 安裝、執行應用程式時給 App 看的文字（ADR-0043）。一句完整的話一則訊息，不在程式碼裡拼；
## 資料（路徑、連接埠、映像檔名、容器名、Docker 的原話）用參數放進句子，不翻。
## 繁體中文用台灣的詞；對使用者叫「應用程式」，不叫「外掛程式」；不說「核心」，說「家用伺服器」。

# ── 進行中的操作（manager.rs busy）──────────────────────────────────────────

plugin-install-busy-installing = 應用程式 { $id } 正在安裝
plugin-install-busy-uninstalling = 應用程式 { $id } 正在解除安裝
plugin-install-busy-upgrading = 應用程式 { $id } 正在更新

# ── 安裝、解除安裝的衝突和權限（manager.rs、install_entry.rs、written_by.rs）──

plugin-install-conflict-already-installed = 應用程式 { $id } 已經安裝了
plugin-install-conflict-slug = 應用程式 { $id } 和已安裝的 { $other } 使用同一個網址
plugin-install-conflict-models-gateway = 已經安裝了模型閘道 { $id }：同一時間只能有一個
plugin-install-conflict-egress-proxy-in-use = 這個代理應用程式正在使用中：先在代理伺服器設定裡換掉，再解除安裝
plugin-install-conflict-lan-rebuilding = 應用程式 { $id } 正在為新的區域網路位址重建
plugin-install-uninstall-unfinished = 解除安裝未完成：{ $error }
plugin-install-permission-not-declared = 授予了清單沒有宣告的權限 { $permission }
plugin-install-permission-unsupported = 家用伺服器還不支援權限 { $permission }
plugin-install-record-unreadable = 無法讀取應用程式的安裝記錄，解除安裝（可以保留資料）後重新安裝：{ $error }
plugin-install-record-detail-failed = 無法產生應用程式 { $id } 的詳細資訊頁
plugin-install-not-agent-written = { $id } 不是 Agent 寫的應用程式，沒有「每次都詢問」的開關

# ── 回復（lifecycle.rs）：失敗清單的項目和彙總 ──────────────────────────────

plugin-install-rollback-item = { $what }：{ $error }
plugin-install-rollback-unfinished = 回復未完成，可能有殘留：{ $failures }
plugin-install-rb-container = 容器 { $name }
plugin-install-rb-home-room = 交給你的主房間 { $room }
plugin-install-rb-bot-device = Bot { $bot } 的裝置
plugin-install-rb-model-credentials = 應用程式 { $id } 的模型閘道憑證
plugin-install-rb-oidc-client = OIDC 用戶端 { $id }
plugin-install-rb-volume = 磁碟區 { $volume }
plugin-install-rb-network = 網路 { $network }
plugin-install-rb-core-leave-network = 家用伺服器退出網路 { $network }
plugin-install-rb-egress-leave-network = 代理容器退出網路 { $network }
plugin-install-rb-directory = 目錄 { $dir }
plugin-install-rb-secret = 應用程式 { $id } 的金鑰

# name_taken 的詞元（lifecycle.rs、leftovers.rs 傳進來的是哪類資源）。
plugin-install-word-volume = 磁碟區
plugin-install-word-network = 網路
plugin-install-word-container = 容器

# ── 部署中的失敗（lifecycle.rs）─────────────────────────────────────────────

plugin-install-oidc-unconfigured = 家用伺服器沒有設定身分提供者（WONTON_OIDC_ADMIN_URL / WONTON_OIDC_PUBLIC_URL）
plugin-install-shared-folder-mkdir-failed = 無法建立共用資料夾目錄 { $dir }：{ $error }
plugin-install-shared-folder-prepare-failed = 無法準備共用資料夾：{ $error }
plugin-install-app-data-prepare-failed = 無法準備應用程式資料目錄：{ $error }
plugin-install-configs-pack-failed = 無法打包 { $service } 的設定檔：{ $error }
plugin-install-teardown-delete-failed = 無法刪除 { $dir }：{ $error }
plugin-install-teardown-leftovers = 解除安裝後仍有殘留：{ $left }

# ── 名稱被占用和陌生資源（leftovers.rs）─────────────────────────────────────

plugin-install-name-taken-owned = { $what } { $name } 屬於應用程式 { $owner }，和這個應用程式的{ $what }同名
plugin-install-name-taken-unknown = { $what } { $name } 已經存在，且不是家用伺服器為應用程式建立的
plugin-install-stranger-attached = 網路 { $network } 上還接著 { $names }，可能是另一套家用伺服器或你自己的容器。一台 Docker 上跑幾套家用伺服器時，每套都要設自己的執行個體名稱（WONTON_INSTANCE）

# ── 對帳和啟動（reconcile.rs、start.rs）─────────────────────────────────────

plugin-install-network-mismatch = 應用程式網路 { $network } { $actual }，但照清單和家用伺服器的運作方式應該{ $expected }：重新安裝應用程式（可以不刪資料）就會生效
plugin-install-resident-stopped = 這個應用程式的常駐服務已經停止
plugin-install-resident-partial = 部分常駐服務還在執行，這些容器已經停止：{ $containers }
plugin-install-containers-unhealthy = 部分容器不健康：{ $failures }
plugin-install-container-unhealthy = 容器 { $container } 健康檢查失敗（最後一次探針，退出碼 { $code }）：{ $output }
plugin-install-container-unhealthy-nocode = 容器 { $container } 健康檢查失敗（最後一次探針，退出碼未知）：{ $output }
plugin-install-container-unhealthy-noprobe = 容器 { $container } 健康檢查失敗（Docker 沒有留下探針結果）
plugin-install-container-health-probe-failed = 容器 { $container } 的健康探測失敗（GET { $path }）：{ $reason }
plugin-install-container-not-running = 容器 { $container } 沒有在執行（{ $status }）
plugin-install-one-shot-failed = 一次性服務的容器 { $container } 沒有以 0 結束（{ $status }，退出碼 { $code }）
plugin-install-reach-none = 不存在
plugin-install-reach-egress = 能連上網際網路
plugin-install-reach-internal = 不能連上網際網路
plugin-install-reach-isolated = 不能連上網際網路、和主機隔開
plugin-install-start-failed = 無法啟動：{ $error }
plugin-install-conflict-rebuilding = 應用程式 { $id } 正在重建，結束後再啟動
plugin-install-record-unusable = 這個應用程式的安裝記錄無法使用，因此無法啟動
plugin-install-container-missing = 容器 { $name } 不存在
plugin-install-container-not-owned = 容器 { $name } 不屬於應用程式 { $id }

# ── 記憶體不足（oom.rs）────────────────────────────────────────────────────

plugin-install-oom-notice = 服務 { $service } 記憶體不足，被系統結束過{ $limit }
plugin-install-oom-reason = 服務 { $service } 記憶體不足，被系統結束{ $limit }
plugin-install-oom-limit-both = （服務上限 { $size }，也可能是所有應用程式加起來達到總上限）
plugin-install-oom-limit-service = （上限 { $size }）
plugin-install-oom-limit-total = （可能是所有應用程式加起來達到總上限）

# ── CPU 和記憶體上限（limits_update.rs、limits.rs 的介面路徑）────────────────

plugin-install-limits-busy = 應用程式 { $id } 正在安裝、解除安裝或重建，等它完成後再試
plugin-install-limits-min-memory = memory 至少要 { $mib } MiB
plugin-install-limits-ncpu-missing = Docker /info 沒有回報 NCPU
plugin-install-limits-ncpu-invalid = Docker /info 回報的 NCPU 不合法
plugin-install-limits-ncpu-out-of-range = Docker /info 的 NCPU 超出範圍
plugin-install-limits-cpu-percent-range = cpu_percent 必須在 1 到 { $max } 之間
plugin-install-limits-host-memory-unknown = 讀不到主機總記憶體，恢復預設要等重建
plugin-install-limits-ncpu-bad = Docker 回報的 CPU 核心數 { $ncpu } 超出範圍
plugin-install-service-error = 服務 { $service }：{ $error }
plugin-install-limits-mem-reservation-order = mem_reservation 不能大於 mem_limit
plugin-install-limits-memory-format = requirements.memory `{ $value }` 不合法
plugin-install-limits-field-forward = { $field }：{ $error }
plugin-install-limits-positive = { $field } 必須大於 0
plugin-install-limits-integer-positive = { $field } 必須是大於 0 的整數
plugin-install-limits-cpus-format = cpus `{ $text }` 不合法
plugin-install-limits-cpus-positive = cpus 必須大於 0
plugin-install-limits-cpus-range = cpus `{ $text }` 超出範圍
plugin-install-limits-memory-range = 記憶體上限超出 Docker 支援的範圍

# ── 應用程式套件（package.rs）───────────────────────────────────────────────

plugin-install-pkg-yaml = compose 檔不是合法的 YAML：{ $error }
plugin-install-pkg-not-mapping = compose 檔的最上層必須是映射
plugin-install-pkg-missing-manifest = compose 檔缺少 `x-wonton` 擴充欄位；沒有清單的 compose 要先補齊權限宣告
plugin-install-pkg-manifest = `x-wonton` 不合法：{ $error }
plugin-install-pkg-service-mapping = 服務定義必須是映射
plugin-install-pkg-unsupported-key = 不支援 `{ $key }`：{ $reason }
plugin-install-pkg-network-mode-unsupported = 不支援 network_mode `{ $mode }`：{ $reason }
plugin-install-pkg-configs-mapping = 必須是映射，寫明 file 或 content
plugin-install-pkg-configs-name-chars = 組態名稱只能包含字母、數字、-、_ 和 .
plugin-install-pkg-configs-key-unsupported = 不支援 `{ $key }`
plugin-install-pkg-configs-file-content-one = file 和 content 只能寫一個
plugin-install-pkg-configs-file-content-required = 必須寫 file 或 content
plugin-install-pkg-configs-file-content-string = file 和 content 必須是字串
plugin-install-pkg-mapping-required = 必須是映射
plugin-install-arch-gzip-failed = 解不開 gzip：{ $error }
plugin-install-arch-unpacked-too-big = 解開後超過 { $mib } MiB
plugin-install-arch-too-many-entries = 壓縮檔裡超過 { $max } 項
plugin-install-pkg-top-level = 最上層欄位 `{ $key }` 不支援：{ $reason }
plugin-install-pkg-service = 服務 `{ $service }`：{ $error }
plugin-install-pkg-interpolate = 服務 `{ $service }` 插值失敗：{ $error }
plugin-install-pkg-unknown-service = { $field } 引用的服務 `{ $service }` 不存在
plugin-install-pkg-volume = 具名磁碟區 `{ $name }` 不合法：只支援不帶參數的本地磁碟區
plugin-install-pkg-config = 組態 `{ $name }`：{ $error }
plugin-install-pkg-config-interpolate = 組態 `{ $name }` 插值失敗：{ $error }
plugin-install-pkg-reference = { $field }：{ $error }
plugin-install-pkg-file = { $field } 引用的檔案 `{ $path }`：{ $error }
plugin-install-pkg-compose-file = 應用程式套件的 compose.yaml：{ $error }
plugin-install-pkg-archive = 應用程式套件封存檔：{ $error }
plugin-install-pkg-too-big = 應用程式套件一共 { $kib } KiB，超過 { $max } KiB 上限
plugin-install-pkg-not-utf8 = 不是 UTF-8 文字
plugin-install-pkg-needs-service = 至少需要一個服務
plugin-install-install-icon-file-missing = { $field } { $raw }：檔案不存在或不是 PNG/SVG
plugin-install-install-icon-error = 圖示 { $path }：{ $reason }
plugin-install-pkg-icon-file = `{ $icon }` 必須是應用程式套件裡的 PNG 或 SVG 檔案
plugin-install-pkg-config-undeclared = 沒有在最上層 configs 中宣告
plugin-install-pkg-config-missing = 應用程式套件裡沒有 `{ $path }`
plugin-install-pkg-ref-bot-service = { $field } 不能指向 Bot 服務：Bot 服務每個 Bot 一份
plugin-install-pkg-egress-network-mode = provides.egress_proxy.service 不能寫 network_mode
plugin-install-pkg-not-in-subset = 不在家用伺服器支援的 Compose 子集裡
plugin-install-pkg-reserved-service = 這個服務名稱是家用伺服器中繼的保留名稱，應用程式不能使用
plugin-install-pkg-service-name-dns = 服務名稱必須是合法的 DNS 標籤：只能用小寫字母、數字和 -
plugin-install-pkg-service-name-dashes = 服務名稱不能以 - 結尾，也不能含 --

# ── 應用程式套件封存檔（package_archive.rs）─────────────────────────────────

plugin-install-arch-read-failed = 讀封存檔失敗：{ $error }
plugin-install-arch-path-invalid = 封存檔裡的路徑不合法：{ $error }
plugin-install-arch-zip-read-failed = 讀 zip 失敗：{ $error }
plugin-install-arch-symlink = 符號連結
plugin-install-arch-zip-path-invalid = zip 裡的路徑不合法：{ $error }
plugin-install-arch-two-roots = { $a } 和 { $b } 裡都有 compose.yaml：一個壓縮檔只能放一個應用程式
plugin-install-arch-missing-compose = 壓縮檔裡沒有 compose.yaml
plugin-install-arch-not-regular = `{ $rel }` 不是一般檔案（{ $what }）：應用程式套件裡不能有符號連結、實體連結和裝置
plugin-install-arch-duplicate-file = 壓縮檔裡有兩個 `{ $rel }`
plugin-install-arch-not-tar = 不是 tar 封存檔：{ $error }
plugin-install-arch-extract-failed = 解不開 `{ $path }`：{ $error }
plugin-install-arch-read-entry-failed = 讀壓縮檔裡的 `{ $path }` 失敗：{ $error }

# ── compose 寫法（compose.rs）───────────────────────────────────────────────

plugin-install-compose-ports-forbidden = 應用程式不對主機開放連接埠，網頁一律經家用伺服器閘道存取
plugin-install-compose-networks-forbidden = 應用程式網路由家用伺服器建立和管理
plugin-install-compose-container-name-forbidden = 容器名稱由家用伺服器分配
plugin-install-compose-build-forbidden = 應用程式只能使用已發布的映像檔，不在主機上建置
plugin-install-compose-env-file-forbidden = 應用程式套件只有一個 compose 檔，環境變數寫進 environment
plugin-install-compose-extends-forbidden = 應用程式套件只有一個 compose 檔，不支援 extends
plugin-install-compose-links-forbidden = 同一應用程式的服務在同一個網路裡，直接用服務名稱互連
plugin-install-compose-external-links-forbidden = 應用程式之間預設無法互相連線
plugin-install-compose-volumes-from-forbidden = 用具名磁碟區或應用程式資料目錄共用資料
plugin-install-compose-top-networks-forbidden = 應用程式網路由家用伺服器建立和管理
plugin-install-compose-top-secrets-forbidden = 金鑰由家用伺服器注入，compose 裡不能宣告 secrets
plugin-install-compose-network-host = 主機網路屬於高權限（network.host）
plugin-install-compose-network-mode = network_mode 只接受 none、host 或 service:<本應用程式的另一個服務>；應用程式網路由家用伺服器建立和管理
plugin-install-compose-config-external-forbidden = 組態只能來自應用程式套件
plugin-install-compose-config-environment-forbidden = 家用伺服器不把自己的環境變數傳給應用程式，需要變數時用 content 插值
plugin-install-compose-config-name-forbidden = 家用伺服器不建立 Docker 組態物件，不需要 name
plugin-install-compose-pair-missing = { $field } 裡的 `{ $key }` 沒有值（家用伺服器不傳遞主機環境變數）
plugin-install-compose-device-format = 裝置映射 `{ $text }` 格式不合法
plugin-install-compose-device-missing-source = 裝置映射缺少 source/path_on_host
plugin-install-compose-device-missing-target = 裝置映射缺少 target/path_in_container
plugin-install-compose-duration-invalid = 時長 `{ $text }` 不合法（範例：30s、1m30s、500ms）
plugin-install-compose-bytes-invalid = 大小 `{ $text }` 不合法（範例：64m、1g）

# ── configs（configs.rs）───────────────────────────────────────────────────

plugin-install-cfg-undeclared = 組態 `{ $name }` 沒有在最上層 configs 中宣告
plugin-install-cfg-target-duplicate = configs 的目標 `{ $raw }` 重複
plugin-install-cfg-target-overlaps-mount = configs 的目標 `{ $raw }` 和掛載點 { $mount } 重疊：組態寫在容器自己的檔案系統裡，不能寫進掛載點
plugin-install-cfg-target-absolute = configs 的目標 `{ $raw }` 必須是絕對路徑，且不能包含 ..
plugin-install-cfg-target-root = configs 的目標不能是 /
plugin-install-cfg-id-invalid = configs 的 { $field } `{ $value }` 必須是非負整數
plugin-install-cfg-mode-invalid = configs 的 mode `{ $value }` 不合法：必須是 0 到 0777 的八進位權限
plugin-install-cfg-parent-is-file = configs 目標的上級 `{ $parent }` 在容器 { $service } 裡是檔案，寫不進去

# ── 掛載（mounts.rs）───────────────────────────────────────────────────────

plugin-install-mnt-short-format = 磁碟區 `{ $text }` 格式不合法
plugin-install-mnt-option-unsupported = 磁碟區選項 `{ $bad }` 不支援
plugin-install-mnt-relative-path = 不支援相對路徑 `{ $path }`，請用 { $var } 底下的路徑
plugin-install-mnt-target-absolute = 掛載目標 { $target } 必須是絕對路徑
plugin-install-mnt-volume-undeclared = 具名磁碟區 { $name } 沒有在最上層 volumes 中宣告
plugin-install-mnt-bind-missing-source = 綁定掛載缺少 source
plugin-install-mnt-host-path-parent = 主機路徑 `{ $path }` 不能包含 ..
plugin-install-mnt-host-path-outside = 主機路徑 `{ $path }` 不在應用程式資料目錄或已授權的共用資料夾裡
plugin-install-mnt-shared-folder-whole = 共用資料夾只能整個掛載，不能只掛其中的 `{ $path }`
plugin-install-mnt-shared-folder-readonly = 共用資料夾 { $path } 只授權了讀取，必須以 :ro 掛載
plugin-install-mnt-nested = 主機路徑 `{ $path }` 在服務 { $outer } 掛載的 { $outer_path } 裡面；同一應用程式的綁定掛載不能嵌套（容器能把內層目錄換成符號連結，逃出應用程式資料目錄），請改成並列的子目錄
plugin-install-mnt-owner-conflict = 和服務 { $other } 以不同的擁有者掛載 { $path }，家用伺服器無法判斷它屬於誰
plugin-install-mnt-numeric-user = 掛載了應用程式資料目錄，user { $user } 要寫成數字形式的 uid:gid：家用伺服器據此決定目錄交給誰

# ── 權限（privileges.rs）───────────────────────────────────────────────────

plugin-install-priv-agent-high-risk = Agent 或 Bot 應用程式不能申請很高或最高權限 { $permission }
plugin-install-priv-fixed-device = 固定裝置權限只支援 /dev/net/tun 和 /dev/fuse，不能申請 { $permission }
plugin-install-priv-host-network-ports = 主機網路服務不能同時宣告 lan.ports；應用程式直接監聽主機網路上的連接埠
plugin-install-priv-host-network-sso = 主機網路裡的 webui 不能使用 sso: proxy，請改用 oidc 或 none
plugin-install-priv-host-network-healthcheck = 主機網路裡的 webui 服務必須寫有效的 healthcheck
plugin-install-priv-host-network-mail = 用主機網路的應用程式不能寄信：寄信按來源網段識別，主機網路寄來的信認不出來源會被丟棄。請改用應用程式網路，或去掉寄信
plugin-install-priv-cap-invalid = cap_add 裡的能力 `{ $cap }` 不合法
plugin-install-priv-cap-drop-invalid = cap_drop 裡的能力 `{ $cap }` 不合法
plugin-install-priv-security-opt-invalid = security_opt 裡的 `{ $opt }` 不合法（no-new-privileges 的值只接受 true 或 false）
plugin-install-priv-device-not-allowed = 裝置 { $source } 不是允許的固定裝置
plugin-install-priv-device-absolute = 裝置目標 { $target } 必須是絕對路徑
plugin-install-priv-device-mode = 裝置 { $source } 的權限 { $permissions } 不合法
plugin-install-priv-pid-host-only = pid 只支援 host
plugin-install-priv-ipc-host-only = ipc 只支援 host
plugin-install-priv-socket-mount = Docker socket 只能以 /var/run/docker.sock:/var/run/docker.sock 可寫掛載
plugin-install-priv-missing-cap-add = 權限 { $permission } 沒有對應的 cap_add 欄位
plugin-install-priv-missing-devices = 權限 { $permission } 沒有對應的 devices 欄位
plugin-install-priv-field-requires = { $field } 需要清單權限 { $permission }
plugin-install-priv-missing-field = 清單權限 { $permission } 沒有對應的 { $field } 欄位
plugin-install-optional-not-allowed = 權限 { $permission } 不能宣告成可選
plugin-install-optional-declared-twice = 權限 { $permission } 同時寫在 permissions 和 optional_permissions 裡
plugin-install-optional-off-loss = 可選權限 { $permission } 要寫 off_loss（關著時少了什麼），中文和英文都要有
plugin-install-optional-gpu-required = 可選權限 { $permission } 關著時不會給顯示卡，requirements.gpu.need 不能寫 required
plugin-install-optional-not-enabled = 可選項 { $permission } 沒有宣告成可選
plugin-install-optional-duplicate = 可選項 { $permission } 寫了兩遍
upgrade-optional-became-required = 新版本把 { $permission } 改成必要權限，確認後授予
upgrade-optional-removed = 新版本去掉了可選項 { $permission }

# ── provides（provides.rs）─────────────────────────────────────────────────

plugin-install-prv-egress-needs-permission = provides.egress_proxy 必須申請 egress.proxy 權限
plugin-install-prv-permission-needs-egress = egress.proxy 權限必須宣告 provides.egress_proxy
plugin-install-prv-egress-needs-network = provides.egress_proxy 必須申請 network.egress 權限
plugin-install-prv-egress-port = provides.egress_proxy.port 不能為 0
plugin-install-prv-empty = provides 至少要宣告一項能力
plugin-install-prv-models-system-only = 只有 kind: system 的應用程式能宣告 provides.models
plugin-install-prv-models-port = provides.models.port 不能為 0
plugin-install-prv-models-apis-path = provides.models.apis 的路徑必須以 / 開頭
plugin-install-prv-models-default-model = provides.models.default_model 不能為空
plugin-install-prv-credentials-argv = provides.models.credentials.{ $field } 必須是非空的 argv，並用 { $placeholder } 指明憑證名稱
plugin-install-prv-list-argv = provides.models.providers.list 必須是非空的 argv
plugin-install-prv-provider-argv = provides.models.providers.{ $field } 必須是非空的 argv，並用 { "{{" }id{ "}}" } 指明供應商 ID
plugin-install-prv-custom-argv = provides.models.providers.custom.{ $field } 必須是非空且不含佔位符的 argv

# ── 代理應用程式的 Cookie 和請求標頭（proxy_auth.rs）────────────────────────

plugin-install-proxy-secret-unknown = { "{{" }secret:{ $name }{ "}}" } 要引用 secrets 裡由家用伺服器產生（generate）、至少 { $min } 位元組的一項
plugin-install-proxy-cookie-template = Cookie 的值只能用 { "{{" }secret:…{ "}}" } 和固定文字：`{ $template }`
plugin-install-proxy-cookie-chars = Cookie 的值裡不能有空白、引號、逗號、分號和反斜線：`{ $template }`
plugin-install-proxy-header-control-chars = 請求標頭的值裡不能有控制字元：`{ $template }`
plugin-install-proxy-header-forbidden = 不能代加請求標頭 { $name }
plugin-install-proxy-cookie-forbidden = 不能代加 Cookie { $name }
plugin-install-proxy-mcp-prefix = mcp.headers：{ $error }
plugin-install-proxy-mcp-header-forbidden = 不能給 MCP 上游加請求標頭 { $name }
plugin-install-proxy-mcp-secret-undeclared = { "{{" }secret:{ $name }{ "}}" } 要引用 secrets 裡宣告的一項
plugin-install-proxy-mcp-identity-placeholder = MCP 標頭不能用 { "{{" }{ $placeholder }{ "}}" } 這類身分佔位符：`{ $template }`
plugin-install-proxy-mcp-header-control-chars = 請求標頭的值裡不能有控制字元：`{ $template }`
plugin-install-proxy-template-open = `{ $template }` 裡的 { "}}" } 沒有配對的 { "{{" }
plugin-install-proxy-template-close = `{ $template }` 裡的 { "{{" } 沒有配對的 { "}}" }
plugin-install-proxy-template-unknown-var = `{ $template }` 裡的 { "{{" }{ $var }{ "}}" } 無法識別：只能用 { "{{" }user{ "}}" }、{ "{{" }name{ "}}" }、{ "{{" }email{ "}}" } 和 { "{{" }secret:<名稱>{ "}}" }
plugin-install-proxy-secret-missing = 應用程式金鑰 { $name } 不在金鑰庫裡
plugin-install-proxy-secret-not-text = mcp.headers 的 { $name } 裡，金鑰 { $secret } 不是文字，放不進請求標頭
plugin-install-proxy-secret-newline = mcp.headers 的 { $name } 裡，金鑰 { $secret } 的內容含換行或控制字元（結尾的換行也算）
plugin-install-proxy-secret-no-value = 應用程式金鑰 { $secret } 還沒有值，mcp.headers 的 { $name } 引用了它
plugin-install-proxy-mcp-no-identity = mcp.headers 的 { $name } 不能用身分佔位符
plugin-install-proxy-mcp-bad-value = mcp.headers 的 { $name } 填出來不是合法的標頭值
plugin-install-proxy-mcp-bad-name = mcp.headers 的 { $name } 不是合法的標頭名稱：{ $error }

# ── 資源需求（requirements.rs、host_check.rs）────────────────────────────────

plugin-install-req-size-format = { $field } `{ $size }` 要寫成 512M、2G 這樣的大小
plugin-install-req-arch-line = 只支援 { $arches } 架構
plugin-install-req-memory-line = 需要 { $memory } 記憶體
plugin-install-req-gpu-vram-suffix = ，顯示卡記憶體 { $memory } 以上
plugin-install-req-gpu-required = 需要{ $card }{ $memory }
plugin-install-req-gpu-optional = 有{ $card }{ $memory }時會用
plugin-install-req-card-generic = 顯示卡
plugin-install-req-card-named = { $names } 顯示卡
plugin-install-or-separator = {" 或 "}
plugin-install-host-unknown = 無法確認{ $what }：{ $why }
plugin-install-host-arch-unknown-what = 這台主機的架構
plugin-install-host-arch-mismatch = 這台主機是 { $arch }
plugin-install-host-memory-wanted-what = 需要多少記憶體
plugin-install-host-memory-unknown-what = 這台主機的記憶體
plugin-install-host-memory-low = 這台主機只有 { $size } 記憶體
plugin-install-host-gpu-unknown-what = 這台主機的顯示卡
plugin-install-host-gpu-problem = { $card }：{ $why }
plugin-install-host-gpu-missing = 這台主機沒有{ $card }
plugin-install-host-fit-vram-unreadable = 無法讀取顯示卡記憶體需求 { $text }
plugin-install-host-fit-unknown = 無法確認顯示卡記憶體：{ $why }
plugin-install-host-fit-integrated = 內建顯示卡和系統共用記憶體，分配到的 { $size } 不是上限，無法確認是否足夠
plugin-install-host-fit-vram-low = 顯示卡記憶體只有 { $size }，但需要 { $text }

# ── 主機情況（host_facts.rs）───────────────────────────────────────────────

plugin-install-facts-gpu-nvidia = NVIDIA { $device }
plugin-install-facts-gpu-amd = AMD { $device }
plugin-install-facts-gpu-intel = Intel { $device }
plugin-install-facts-gpu-other = 顯示裝置 { $vendor }:{ $device }
plugin-install-facts-meminfo-missing = /proc/meminfo 裡沒有 { $field }
plugin-install-facts-meminfo-unreadable = 讀不了 /proc/meminfo：{ $error }
plugin-install-facts-docker-not-reported = Docker 沒有回報
plugin-install-facts-pci-unreadable = 讀不了 { $path }：{ $error }
plugin-install-facts-driver-nouveau = 用的是 nouveau 驅動，不能用於運算，需要 NVIDIA 官方驅動
plugin-install-facts-driver-bound = 綁定的是 { $driver } 驅動
plugin-install-facts-driver-none = 沒有載入驅動
plugin-install-facts-runtime-missing = Docker 沒有登記 nvidia 執行階段：要安裝 NVIDIA Container Toolkit
plugin-install-facts-runtime-unknown = 不知道 Docker 有沒有 nvidia 執行階段：{ $reason }
plugin-install-facts-driver-no-render = { $driver } 驅動沒有渲染節點，不能用於運算
plugin-install-facts-no-host-dev = 家用伺服器在容器裡，但沒有掛載主機的 /dev（WONTON_HOST_DEV），看不到裝置節點
plugin-install-facts-dri-missing = 主機的 /dev/dri 裡沒有 { $render }
plugin-install-facts-model-unknown = 驅動沒有給出型號，只知道裝置 ID
plugin-install-facts-vram-not-in-sysfs = NVIDIA 的顯示卡記憶體不在 sysfs 裡
plugin-install-facts-vram-unknown = 驅動沒有給出顯示卡記憶體大小
plugin-install-facts-core-not-in-container = 家用伺服器不在容器裡，沒有可以用來向 NVIDIA 驅動查詢的映像檔
plugin-install-facts-container-trial-failed = 在容器裡試執行失敗：{ $reason }
plugin-install-facts-container-image-unknown = 查不到家用伺服器容器 { $core } 的映像檔
plugin-install-facts-nvidia-smi-exit = nvidia-smi 以 { $code } 結束：{ $reason }

# ── 顯示卡（gpu.rs）────────────────────────────────────────────────────────

plugin-install-gpu-needs-permission = 宣告了 gpu 就必須申請 devices.gpu 權限
plugin-install-gpu-needs-services = 申請了 devices.gpu 就必須用 gpu.services 寫明哪些服務用顯示卡
plugin-install-gpu-needs-all-three = requirements.gpu 要和 devices.gpu 權限、gpu.services 一起宣告
plugin-install-gpu-services-nonempty = gpu.services 至少一項，且不能重複
plugin-install-gpu-describe-vendors = { $vendors } 顯示卡：{ $devices }，容器加入群組 { $groups }
plugin-install-gpu-describe-nvidia = NVIDIA 顯示卡：全部，經 NVIDIA Container Toolkit
plugin-install-gpu-problem-host-dev = 家用伺服器在容器裡，但沒有掛載主機的 /dev（WONTON_HOST_DEV），看不到顯示卡裝置：用目前版本的 compose 檔重新啟動家用伺服器
plugin-install-gpu-problem-unreadable = 讀不了 { $path }：{ $error }
plugin-install-gpu-problem-no-device = 找不到顯示卡（PCI 上沒有顯示裝置）
plugin-install-gpu-problem-no-driver = 有顯示卡但沒有可用的驅動：{ $detail }
plugin-install-gpu-problem-no-runtime = 有 NVIDIA 驅動，但 Docker 沒有登記 nvidia 執行階段：請安裝 NVIDIA Container Toolkit 並執行 `nvidia-ctk runtime configure --runtime=docker`
plugin-install-gpu-problem-no-wanted-vendor = 沒有{ $card }
plugin-install-gpu-vendor-other = 廠商 { $id }
plugin-install-gpu-no-driver-none = 沒有載入驅動
plugin-install-gpu-no-driver-nouveau = 用的是 nouveau 驅動，不能用於運算，需要 NVIDIA 官方驅動
plugin-install-gpu-no-driver-no-render = 載入了 { $driver } 驅動，但沒有渲染節點
plugin-install-gpu-device-entry = { $address }（{ $vendor }）{ $state }
plugin-install-gpu-no-device-nodes = 裡面沒有裝置節點
plugin-install-gpu-preview-granted = 顯示卡交給服務 { $services }：{ $describe }
plugin-install-gpu-preview-missing = 本機沒有符合的顯示卡（{ $problem }），服務 { $services } 裝上後不帶顯示卡執行
plugin-install-gpu-preview-required = 服務 { $services } 需要顯示卡，{ $reason }

# ── 渲染（render.rs）───────────────────────────────────────────────────────

plugin-install-render-no-owner-identity = 應用程式沒有申請 owner.identity，不能引用 `{ $var }`：在清單的 permissions 裡加上它
plugin-install-render-bot-dependency = 全應用程式共用的服務不能依賴 Bot 服務 `{ $service }`：Bot 服務每個 Bot 一份
plugin-install-render-one-shot-restart = 被 service_completed_successfully 依賴的一次性服務必須寫 restart: "no"：否則 Docker 會在它結束後反覆重新啟動
plugin-install-render-network-mode-none = { $field } 指向的服務不能寫 network_mode: none
plugin-install-render-dependency-missing = depends_on 引用了不存在的服務 `{ $missing }`
plugin-install-render-dependency-cycle = depends_on 有循環相依
plugin-install-render-reserved-label = `wonton.` 開頭的標籤由家用伺服器保留
plugin-install-render-quotes-unbalanced = 命令 `{ $text }` 的引號不配對
plugin-install-render-restart-invalid = restart 的值 `{ $value }` 不合法
plugin-install-render-healthcheck-format = healthcheck.test 清單必須以 CMD、CMD-SHELL 或 NONE 開頭

# ── Docker（docker.rs、docker_exec.rs、core_address.rs）──────────────────────

plugin-install-docker-unavailable = 無法連上 Docker：{ $error }
plugin-install-docker-error-join = { $context }：{ $error }
plugin-install-docker-timed-out = Docker 沒有按時處理完（{ $context }），操作可能還在進行
plugin-install-docker-not-found = 在 Docker 裡找不到：{ $what }
plugin-install-docker-rejected = Docker 拒絕了請求（{ $context }）：{ $error }
plugin-install-port-taken = 連接埠 { $place } 已被佔用，停掉佔著它的容器或程式再試。（{ $error }）
plugin-install-port-taken-unknown = 啟動容器時連接埠已被佔用，停掉佔著它的容器或程式再試。（{ $error }）
plugin-lan-taken-by-plugin = 應用程式 { $id }
plugin-lan-taken-by-container = 容器 { $name }
plugin-lan-taken-by-host = 主機上的程式
plugin-install-lan-port-taken = 區域網路連接埠 { $place } 已被{ $who }佔用，停掉它再裝。
plugin-install-lan-port-hinted = 區域網路連接埠 { $place } 已被{ $who }佔用，裝到啟動那一步會失敗。
plugin-install-lan-port-probe-failed = 沒能檢查主機上的程式是否佔用這些連接埠：{ $reason }
plugin-install-probe-image-replaced = 家用伺服器的映像檔已被替換，檢查容器用的是它建立時的映像檔；重新啟動家用伺服器後恢復（{ $reason }）
plugin-install-probe-image-missing = 家用伺服器的映像檔已經不在 Docker 裡了，檢查容器起不來；重新啟動家用伺服器後恢復。Docker 的原話：{ $error }
plugin-install-docker-pull-failed = 拉取映像檔失敗（{ $image }）：{ $error }
plugin-install-docker-context-version = 查詢版本
plugin-install-docker-context-host-info = 查詢 Docker 資訊
plugin-install-docker-context-image-healthcheck = 檢查映像檔 { $image }
plugin-install-docker-context-network = 查詢網路 { $name }
plugin-install-docker-no-ipv4-gateway = 網路沒有 IPv4 閘道
plugin-install-docker-no-subnets = 網路沒有位址範圍
plugin-install-docker-context-remove-network = 刪除網路 { $name }
plugin-install-docker-context-list-published-ports = 列出容器已發佈的連接埠
plugin-install-docker-context-connect-network = 把 { $container } 接上網路 { $network }
plugin-install-docker-context-disconnect-network = 把 { $container } 移出網路 { $network }
plugin-install-docker-context-create-volume = 建立磁碟區 { $name }
plugin-install-docker-context-volume-labels = 查詢磁碟區 { $name }
plugin-install-docker-context-remove-volume = 刪除磁碟區 { $name }
plugin-install-docker-context-create-container = 建立容器 { $name }
plugin-install-docker-context-upload-archive = 把檔案寫進容器 { $container }
plugin-install-docker-context-container-missing = 容器 { $container }
plugin-install-docker-context-read-file = 讀取容器 { $container } 裡的 { $path }
plugin-install-docker-archive-unparseable = 封存檔無法解析：{ $error }
plugin-install-docker-context-start-container = 啟動容器 { $name }
plugin-install-docker-context-inspect-container = 查詢容器 { $name }
plugin-install-docker-context-kill-container = 對容器 { $name } 發送 { $signal }
plugin-install-docker-context-stop-container = 停止容器 { $name }
plugin-install-docker-context-remove-container = 刪除容器 { $name }
plugin-install-docker-context-list-containers = 列出容器
plugin-install-docker-context-list-networks = 列出網路
plugin-install-docker-context-list-volumes = 列出磁碟區
plugin-install-docker-engine-too-old = Docker Engine { $version } 太舊，家用伺服器需要 { $min } 以上：從 28 版起，區域網路裡的其他主機不能再直接存取容器裡沒發布的連接埠，應用程式網路也才能和主機完全隔開。請更新 Docker
plugin-install-docker-engine-version-unrecognized = 無法識別 Docker Engine 的版本 `{ $version }`
plugin-install-docker-context-exec = 在容器 { $container } 裡執行 { $argv }
plugin-install-docker-exec-no-io = 命令沒有接上輸入輸出
plugin-install-docker-exec-timed-out = { $context }：{ $seconds } 秒還沒結束
plugin-install-coreaddr-no-ipv4-layout = 網路沒有能給家用伺服器留位址的 IPv4 網段
plugin-install-coreaddr-pool-exhausted-context = 建立網路 { $name } 時，家用伺服器給應用程式網路保留的位址範圍用完了，應用程式裝不上；解除安裝不用的應用程式，或把家用伺服器設定 { $var } 裡的位址範圍改大
plugin-install-coreaddr-pool-exhausted = 位址範圍 { $subnet }，已用 { $count }/{ $count } 個 /26 網段
plugin-install-coreaddr-kept-taken = { $context }，{ $attempts } 次都和同時建立的網路撞了網段
plugin-install-coreaddr-reserved-taken = 保留的網段 { $subnet } 被其他程式佔用了：重試這次重建，會另挑網段
plugin-install-coreaddr-context-create-network = 建立網路 { $name }
plugin-install-coreaddr-context-inspect-network = 查詢網路 { $name }
plugin-install-coreaddr-no-core-address = 網路還沒有為家用伺服器保留位址

# ── 上游和區域網路（upstreams.rs、host_relay.rs）────────────────────────────

plugin-install-upstream-container-missing = 容器 { $container } 不存在，或者在網路 { $network } 裡沒有位址
plugin-install-upstream-mcp-headers = 無法解析應用程式 { $id } 的 MCP 請求標頭：{ $error }
plugin-install-relay-prepare-dir-failed = 無法準備中繼目錄：{ $error }
plugin-install-relay-path-too-long = 中繼 socket 的目錄路徑太長，unix socket 放不下，把 WONTON_RELAY_DIR 換到短一點的目錄（{ $path }：{ $bytes } 位元組，上限 { $max } 位元組）
plugin-install-relay-listens-unknown = 讀不到監聽表，略過防火牆判斷
plugin-install-egress-host-no-address = 出口容器在插件網路 { $network } 裡沒有位址，宿主網路的服務過不了代理

# ── 圖片（images.rs、images/preflight.rs、image_worker.rs）──────────────────

plugin-install-img-tab-icon-alpha = 底欄圖示的透明色版不能全不透明或全透明
plugin-install-img-too-many-kib = 超過 { $kib } KiB
plugin-install-img-too-many-mib = 超過 { $mib } MiB
plugin-install-img-not-png-or-jpeg = 不是 PNG 或 JPEG 圖片
plugin-install-img-is-format = 這是 { $format } 圖片，螢幕截圖只收 PNG 或 JPEG
plugin-install-img-encode-jpeg-failed = 編碼 JPEG 失敗：{ $error }
plugin-install-img-not-supported-format = 不是支援的圖片格式
plugin-install-img-format-no-thumbnail = { $format } 做不了縮圖
plugin-install-img-over-edge-or-memory = 超過 { $max }×{ $max } 像素或記憶體上限
plugin-install-img-decode-failed = 解碼失敗：{ $error }
plugin-install-img-png-too-big = PNG 圖示超過 { $kib } KiB
plugin-install-img-png-content = 檔名是 .png，內容卻不是 PNG
plugin-install-img-not-square = 圖示必須是正方形，這張是 { $width }×{ $height }
plugin-install-img-png-too-small = PNG 圖示至少 { $min }×{ $min }，這張是 { $width }×{ $height }
plugin-install-img-svg-too-big = SVG 圖示超過 { $kib } KiB
plugin-install-img-svg-not-utf8 = SVG 必須是 UTF-8 文字（不收壓縮過的 SVG）
plugin-install-img-svg-invalid = 不是合法的 SVG：{ $error }
plugin-install-img-svg-parse-failed = 解析 SVG 失敗：{ $error }
plugin-install-img-svg-viewbox = SVG 的 viewBox 必須是正方形，這張是 { $width }×{ $height }
plugin-install-img-canvas-alloc-failed = 無法分配畫布
plugin-install-img-canvas-convert-failed = 無法轉換畫布
plugin-install-img-over-max-edge = 超過 { $max }×{ $max } 像素
plugin-install-img-encode-png-failed = 編碼 PNG 失敗：{ $error }
plugin-install-img-svg-root = 根元素必須是 <svg xmlns="{ $ns }">
plugin-install-img-style-rules = 樣式表最多 { $max } 條規則，這裡有 { $count } 條
plugin-install-img-style-combinator = 樣式表的選擇器「{ $selector }」帶了組合符：只能用 .cls-1 這樣不帶組合符的選擇器
plugin-install-img-nesting-depth = SVG 巢狀超過 { $max } 層
plugin-install-img-use-cycle = SVG 裡的 <use> 循環引用
plugin-install-img-use-expanded = SVG 展開 <use> 後超過 { $max } 個元素
plugin-install-img-use-href-count = <use> 要有且只有一個 href（href 或 xlink:href）
plugin-install-img-use-href-format = <use> 的 href 只能寫成 #id，id 裡不能有空白：{ $href }
plugin-install-img-element-forbidden = 圖示不能用 <{ $name }>
plugin-install-img-clip-path-units = <clipPath> 的 clipPathUnits 只能是 userSpaceOnUse
plugin-install-img-mask-units = <mask> 要寫 maskUnits="userSpaceOnUse"，maskContentUnits 只能是 userSpaceOnUse
plugin-install-img-gradient-stops = 漸層最多 { $max } 個 <stop>，這個有 { $count } 個
plugin-install-img-worker-usage = 用法：wonton-core { $command } <icon.png|icon.svg|tab-icon.png|tab-icon.svg|screenshot|thumbnail|preview>（家用伺服器內部用）
plugin-install-img-worker-unavailable = 處理圖片的行程無法使用：{ $error }
plugin-install-img-worker-rlimit = 設不了資源上限：{ $error }
plugin-install-img-worker-stdin = 讀標準輸入失敗：{ $error }
plugin-install-img-worker-stdout = 寫標準輸出失敗：{ $error }
plugin-install-img-worker-spawn-failed = 無法啟動 { $program }：{ $error }
plugin-install-img-worker-communicate = 和子行程通訊失敗：{ $error }
plugin-install-img-worker-timeout = 處理超過 { $seconds } 秒，已停止
plugin-install-img-worker-crashed = 處理時異常結束（結束代碼 { $code }）
plugin-install-img-worker-cpu-timeout = 處理超過 { $seconds } 秒 CPU 時間，已停止
plugin-install-img-worker-memory-limit = 處理時需要的記憶體超過 { $mib } MiB
plugin-install-img-worker-killed = 處理圖片的行程被強制終止（SIGKILL），多半是記憶體不足
plugin-install-img-worker-signal = 處理圖片的行程被訊號 { $signal } 結束
plugin-install-img-worker-malformed = 處理圖片的行程輸出的格式不對
plugin-install-img-worker-host-exited = 處理圖片的助手退出了：{ $error }
plugin-install-img-worker-host-not-ready = 處理圖片的助手沒有就緒：{ $error }
plugin-install-img-worker-host-protocol = 處理圖片的助手用的協定版本是 { $got }，核心是 { $want }

## 预装（启动时随服务器带的包）失败的原因。
plugin-install-preinstall-retrying = 預先安裝失敗（已試 { $attempts } 次，稍後自動重試）
plugin-install-preinstall-failed = 預先安裝失敗

# ── 能力依賴（capabilities.rs，ADR-0053）──────────────────────────────────

plugin-capability-unknown = 這份核心不認識能力 { $capability }，升級核心後再裝
plugin-capability-third-party-format = 能力 { $capability } 不合法：第三方能力要寫成 x-<作者>.<名字>，作者和名字用小寫字母、數字、連字符號
plugin-capability-no-consumer = { $capability } 沒有使用方聲明，不能寫在 requires 或 wants 裡
plugin-capability-duplicate = { $field } 裡的能力 { $capability } 寫了兩次
plugin-capability-in-both-lists = 能力 { $capability } 不能同時在 requires 和 wants 裡
plugin-capability-provider-requires = 提供能力的插件不能 requires 別的能力
plugin-capability-port-zero = provides.{ $capability }.port 不能為 0
plugin-capability-credentials-required = provides.{ $capability } 要宣告 credentials：給每個使用方簽一個只屬於它的憑據
plugin-capability-credentials-forbidden = provides.{ $capability } 不能宣告 credentials：這個能力的提供方只認經核心轉發來的請求
plugin-capability-credentials-argv = provides.{ $capability }.credentials.{ $field } 是非空的 argv，並用 { $placeholder } 指明憑據名
plugin-capability-service-unknown = { $field } 引用的服務 `{ $service }` 在 compose 裡不存在
plugin-capability-service-not-forwardable = { $field } 不能指向 network_mode: none 的服務：核心要轉發進去、在裡面簽發憑據
plugin-install-capability-deps-unsupported = 這個核心還不支援外掛之間的依賴，升級核心後再裝
plugin-install-runtime-services = runtime 為 { $runtime } 時不能宣告 services
plugin-install-runtime-unsupported = 這個核心不支援安裝或升級 runtime 為 { $runtime } 的外掛，請先升級核心
plugin-install-matrix-rtc-needs-matrix = 核心沒有接入 Matrix 伺服器，裝不了提供 matrix_rtc（通話授權）的應用程式
plugin-install-matrix-rtc-conflict = 已經安裝了提供 matrix_rtc（通話授權）的應用程式 { $id }：同一時間只能有一個
plugin-install-matrix-rtc-ca-failed = 寫不了通話授權應用程式要信任的憑證包：{ $error }
plugin-install-capability-dropped = 宿主網路的服務預設被核心去掉了 { $capability }。映像要用（ICMP ping 之類）就申請 { $permission } 權限，並給服務 { $service } 寫 { $field }。

# 静态网页和托管形态（ADR-0080）
plugin-install-static-dir = `{ $root }` 裡的靜態檔案：{ $problem }
plugin-install-static-too-many = 檔案和資料夾超過 { $max } 個
plugin-install-static-too-big = 一共 { $mib } MiB，超過 { $max } MiB
plugin-install-static-file-too-big = `{ $path }` 超過單一檔案 { $max } MiB 的上限
plugin-install-static-hidden = `{ $path }`：不能有以 . 開頭的名稱
plugin-install-static-reserved = _wonton/ 目錄保留給應用程式資料，不能使用
plugin-install-static-no-index = 沒有 index.html
plugin-install-static-no-fallback = fallback 指定的 `{ $path }` 不在目錄裡
plugin-install-static-symlink = `{ $path }` 是符號連結，靜態檔案不能是符號連結
plugin-install-static-not-file = `{ $path }` 不是一般檔案
plugin-install-static-missing = 目錄不存在
plugin-install-static-read-failed = 無法讀取 `{ $path }`：{ $error }
plugin-hosted-provider-not-running = 託管這個應用程式的 { $provider } 沒有在執行
plugin-hosted-provider-missing = 沒有安裝提供 { $capability } 的應用程式，這個應用程式無法使用
plugin-hosted-provider-taken = { $other } 已經提供 { $capability }，只能有一個應用程式提供它
plugin-hosted-provider-in-use = 這些應用程式由它託管，解除安裝後會打不開：{ $plugins }。請先解除安裝它們
plugin-hosted-dir-missing = 這個應用程式在 { $capability } 下的檔案不見了，請重新安裝
plugin-hosted-storage-failed = 託管檔案無法寫入：{ $error }
