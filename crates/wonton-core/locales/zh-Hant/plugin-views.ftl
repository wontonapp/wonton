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
permission-models = 使用你接好的模型
permission-tools = 呼叫家用伺服器和其他應用程式提供的小工具
permission-plugins-health = 查看所有應用程式是否正常
permission-matrix-bot = 以聊天機器人身分收發訊息
permission-notify = 傳送通知和信件給家人
permission-owner-identity = 知道你的使用者名稱和電子郵件，也能查詢家人使用的語言
permission-network-egress = 存取外部網路和區域網路
permission-egress-proxy = 幫其他應用程式連外（高權限）
permission-network-lan = 讓家裡的裝置直接連它（高權限）
permission-network-clients = 你打開開關後，讓別的應用程式從家裡網路或網際網路連進來
permission-network-host = 直接用家用伺服器的網路（很高權限）
permission-capability = 增加系統能力 { $name }
permission-host-pid = 能看到伺服器上正在執行的所有程式（最高權限）
permission-host-ipc = 和伺服器上的程式直接交換資料（最高權限）
permission-security-opt = 改它的安全設定（最高權限）
permission-privileged = 用最大的權限執行（最高權限）
permission-devices-gpu = 使用顯示卡（高權限）
permission-device = 使用伺服器上的裝置 { $path }（高權限）
permission-devices-serial = 使用伺服器上的序列埠裝置（高權限）
permission-devices-usb = 使用伺服器上的 USB 裝置（高權限）
permission-devices-disk = 讀寫你勾選的整顆硬碟（很高權限）
permission-docker-socket = 管理 Docker（最高權限）

# 高權限下面帶的長說明。
permission-notice-egress-proxy = 選它當代理伺服器後，其他應用程式連外都要經過它：它能看到這些應用程式上了哪些網站，也能改掉未加密的內容。
permission-notice-network-host = 這個應用程式能像伺服器自己一樣上網，包括只給伺服器用的服務，還能繞過雲吞的登入連到部分其他應用程式。它開的連接埠在所有網路位址上都連得到，家裡網路和網際網路（伺服器有公網位址時）都一樣；伺服器開著防火牆時，家裡的裝置連不進來，要你放行。
permission-notice-highest-risk = 這等於把伺服器的最高控制權交給它：能讀寫所有檔案和其他應用程式的資料，能改動或停掉雲吞自己。只安裝你信任的應用程式；解除安裝後，它留下的容器和改過的東西不會被清理。
permission-notice-network-lan = 家裡的裝置能直接連它開的連接埠，不用登入雲吞；誰能進來由它自己決定。
permission-notice-device = 這個應用程式能使用伺服器上的裝置 { $path }。
permission-notice-devices = 這個應用程式能使用你交給它的伺服器裝置。
permission-notice-devices-disk = 這個應用程式能讀寫你勾選的硬碟上的全部資料。
permission-notice-devices-usb = 這個應用程式能使用伺服器上所有 USB 裝置，包括隨身碟和讀卡機。

# 應用程式詳情裡的提示。
plugin-firewall-notice = 本機防火牆可能擋住了外來連線，區域網路裝置可能連不進來
plugin-egress-proxy-select = 在代理伺服器設定裡選它
plugin-cross-origin-isolation = 這個應用程式需要跨來源隔離：在 Android 上，它的頁面會在瀏覽器裡開啟。

# 安裝和更新預覽裡的提示與裝不了的原因。
install-retained-data = 伺服器上還留著 { $id } 以前的資料和密碼，裝上後讀得到；不想這樣就先徹底刪掉
install-image = { $service }（{ $image }）
install-images-from = 程式來自 { $images }
install-images-unpinned = 程式沒有鎖定具體版本：{ $images }
install-webui-sso = 在應用程式裡開啟它的網頁時自動登入
install-host-network-egress = 它連外用伺服器自己的網路，不經過雲吞的代理伺服器
install-host-network-proxied = 它連外也經過雲吞的代理伺服器
install-already-installed = 已經安裝了 { $id }（版本 { $version }）
install-unreviewed-provider = 雲吞沒有審核過這個應用程式，程式由 { $provider } 提供
install-unreviewed-provider-via = 雲吞沒有審核過這個應用程式，程式由 { $provider } 提供，列在 { $host } 裡
install-unreviewed-upload-author = 雲吞沒有審核過這個應用程式，它來自你上傳的檔案，清單裡寫的作者是 { $author }，無法核實
install-unreviewed-upload = 雲吞沒有審核過這個應用程式，它來自你上傳的檔案，清單裡沒寫作者
plugin-widget-high = 這個應用程式的網頁能在你開啟它的房間裡讀到聊天內容，或以你的名義傳訊息、收回訊息。
plugin-widget-normal = 這個應用程式的網頁還能收發通話和給裝置的訊息、定時發出事件、查通話伺服器，並拿到你的登入憑證。
