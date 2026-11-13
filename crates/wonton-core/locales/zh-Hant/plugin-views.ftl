## 伺服器發給 App 的文字（ADR-0043）。一條完整的話一條訊息，不在程式碼裡拼；
## 資料（路徑、連接埠、裝置名稱、錯誤碼）作參數嵌進句子，不翻。
## 台灣用詞，引號「」，中英文之間加空格；對使用者叫「應用程式」，不叫「外掛程式」。

# 列舉資料項（映像檔倉庫這些）用的分隔符。
list-separator = 、

# 權限：安裝確認頁、Bot 的確認卡片和商店搜尋結果都用它。
permission-storage-share =
    { $access ->
        [read] 讀取共用資料夾 { $name }
       *[write] 讀寫共用資料夾 { $name }
    }
permission-storage-home =
    { $access ->
        [read] 讀取你的私人空間
       *[write] 讀寫你的私人空間
    }
permission-models = 使用模型閘道
permission-tools = 呼叫家用伺服器和其他應用程式的工具
permission-plugins-health = 讀取所有應用程式的健康狀態
permission-matrix-bot = 以 Bot 身分收發訊息
permission-notify = 傳送通知和信件給家人
permission-owner-identity = 知道你的使用者名稱和電子郵件，也能查詢家人使用的語言
permission-network-egress = 存取外部網路和區域網路
permission-egress-proxy = 提供代理伺服器（高權限）
permission-network-lan = 在區域網路上開放連接埠（高權限）
permission-network-clients = 讓別的 App 從區域網路或公網連進來，由你打開開關
permission-network-host = 使用主機的網路（很高權限）
permission-capability = 增加核心能力 { $name }
permission-host-pid = 共用主機的行程空間（最高權限）
permission-host-ipc = 共用主機 IPC（最高權限）
permission-security-opt = 修改容器安全選項（最高權限）
permission-privileged = 以特權模式執行（最高權限）
permission-devices-gpu = 使用 GPU（高權限）
permission-device = 存取主機裝置 { $path }（高權限）
permission-devices-serial = 存取主機序列埠裝置（高權限）
permission-devices-usb = 存取主機 USB 裝置（高權限）
permission-docker-socket = 控制 Docker（最高權限）

# 高權限下面帶的長說明。
permission-notice-egress-proxy = 選它做代理伺服器後，所有會連外的應用程式，請求都會經過它：它能看到這些應用程式造訪了哪些網站，也能改寫未加密的內容。
permission-notice-network-host = 這個應用程式能直接存取你家裡的網路和本機所有連接埠，包括只給本機用的服務，也能繞過 Wonton 的登入連到部分其他應用程式。它開的連接埠會出現在本機所有網路卡上，包括 Wi-Fi、Tailscale 和 IPv6 位址，本機有公網位址時網際網路也連得上。本機開著防火牆時，區域網路的裝置連不進來，要你自己放行。
permission-notice-highest-risk = 這個應用程式能控制整台主機：讀寫所有檔案和其他應用程式的資料，改動或停用 Wonton 自己。這等於把主機的 root 交給它。只安裝你信任的應用程式。解除安裝後，它建立的容器和改過的東西不會被清理。
permission-notice-network-lan = 區域網路裡的裝置可以直接連接應用程式開放的連接埠，不經 Wonton 登入；身分驗證由應用程式自己負責。
permission-notice-device = 這個應用程式可以存取主機裝置 { $path }。
permission-notice-devices = 這個應用程式可以存取你交給它的主機裝置。

# 應用程式詳情裡的提示。
plugin-firewall-notice = 本機防火牆可能擋住了外來連線，區域網路裝置可能連不進來
plugin-egress-proxy-select = 在代理伺服器設定裡選它
plugin-cross-origin-isolation = 這個應用程式需要跨來源隔離：在 Android 上，它的頁面會在瀏覽器裡開啟。

# 安裝和更新預覽裡的提示與裝不了的原因。
install-retained-data = 這台伺服器上留著 { $id } 以前的資料和金鑰，這個應用程式裝上後能讀到它們；不想這樣就先徹底刪除
install-image = { $service }（{ $image }）
install-images-from = 映像檔來自 { $images }
install-images-unpinned = 映像檔沒有按摘要固定：{ $images }
install-webui-sso = 在雲吞裡開啟它的網頁時自動登入
install-host-network-egress = 主機網路裡的服務連外走主機自己的路由，不經 Wonton 的代理伺服器
install-host-network-proxied = 主機網路裡的服務也經你的出網代理連外
install-system-preinstalled = 系統應用程式只能隨家用伺服器預先安裝
install-already-installed = 已經安裝了 { $id }（版本 { $version }）
install-unreviewed-provider = 雲吞沒有審核過這個應用程式，程式碼和映像檔由 { $provider } 提供
install-unreviewed-provider-via = 雲吞沒有審核過這個應用程式，程式碼和映像檔由 { $provider } 提供，經 { $host } 的索引
install-unreviewed-upload-author = 雲吞沒有審核過這個應用程式，它來自你上傳的檔案，清單裡寫的作者是 { $author }，無法核實
install-unreviewed-upload = 雲吞沒有審核過這個應用程式，它來自你上傳的檔案，清單裡沒寫作者
plugin-widget-high = 這個應用程式的網頁能在你開啟它的房間裡讀到聊天內容，或以你的名義傳訊息、收回訊息。
plugin-widget-normal = 這個應用程式的網頁還能收發通話和給裝置的訊息、定時發出事件、查通話伺服器，並拿到你的登入憑證。
