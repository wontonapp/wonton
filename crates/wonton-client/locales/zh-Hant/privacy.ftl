privacy-what-title = 沒有雲
privacy-what-own = 雲吞沒有自己的雲：伺服器就是你家（或你自己租的）那台機器，訊息、照片、檔案都存在它上面。
privacy-what-none = 我們收不到帳號、裝置資訊，App 裡也沒有統計、廣告和當機回報。

privacy-messages-title = 聊天和檔案
privacy-messages-e2ee = 聊天和檔案都端對端加密，能解開的只有你的裝置。
privacy-messages-history = 歷史存在你自己的伺服器上，備份金鑰由你保管。

privacy-push-title = 通知（僅 Android）
privacy-push-keepalive = 用「保持連線」時，通知不走外面，隧道的另一頭就是你家。
privacy-push-unified = 改用 UnifiedPush 後，家裡的 Core 先用你裝置的公鑰把通知加密，再交給分發器；分發器只看得到密文，到手機上才解開。分發器是第三方服務。

privacy-network-title = 怎麼連上家裡
privacy-network-lan = 同一個區域網路時直連；不在一個網路時先打洞，直連不通才經中繼轉發。
privacy-network-relay = 中繼只轉發加密後的流量，看不到內容。預設用 iroh 的公共中繼，你家可以換成自己的。

privacy-preview-title = 連結預覽
privacy-preview-core = 聊天裡出現連結時，由你家的 Core 去取網頁的標題和圖片，網站看到的是你家伺服器的位址。

privacy-plugins-title = 第三方登入和插件網頁
privacy-plugins-oidc = 用別的帳號登入插件時，帳號和密碼只發給那個插件的伺服器，家裡的 Core 不保存第三方密碼。
privacy-plugins-web = 插件的網頁在你手機的瀏覽器核心裡打開，它能不能出網、出到哪，由插件自己決定。

privacy-device-title = 留在手機上的東西
privacy-device-files = 裝置金鑰、連線資訊、快取和日誌放在 App 的私有目錄裡，別的 App 讀不到；移除 App 時一起刪掉，伺服器上的歷史不受影響。
privacy-device-backup = 這些檔案會跟著系統備份走（iCloud 或 Google 備份），備份存在哪個帳號裡由系統的設定決定。
