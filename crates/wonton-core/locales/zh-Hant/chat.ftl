## 家用伺服器傳進聊天的文字（ADR-0043）：確認卡片和它的結果行、給其他 Matrix 用戶端看的
## 純文字卡片、Bot 房間交不出去時的踢出理由、Agent 轉述給你的說明文字。一句完整的話一則
## 訊息；資料（Bot 名字、應用程式名稱、版本、路徑、上游的錯誤原文）當參數放進句子，不翻。

# 確認卡片上的按鈕。
chat-choice-install = 安裝
chat-choice-deny-install = 不安裝
chat-choice-update = 更新
chat-choice-deny-update = 不更新
chat-choice-uninstall = 解除安裝
chat-choice-cancel = 取消
chat-choice-allow = 允許
chat-choice-deny = 拒絕

# 工具確認卡片上的放行選項：一段時間內再呼叫不再問。參數是整數的分鐘數或秒數。
chat-card-window-minutes = { $minutes } 分鐘內都允許
chat-card-window-seconds = { $secs } 秒內都允許

# 安裝和更新卡片上的行。$requester 是 Bot 的名字，$title 是應用程式的名字。
chat-card-image = 服務 { $service } 使用映像檔 { $image }
chat-card-ports-added = 新增區域網路連接埠 { $ports }
chat-card-gpu-added = 新增使用 GPU 的服務 { $services }
chat-card-version-official = 版本 { $version }，來自官方索引
chat-card-version = 版本 { $version }
chat-card-update-from = 從版本 { $from } 更新到 { $to }
chat-card-no-permissions = 不需要額外的權限
chat-card-permissions-header = 要授予的權限：
chat-card-agent-source = 來自你的 Agent · { $requester }，未經審核：程式由 Agent 撰寫，由你把關
chat-card-agent-ask-again = 以後 { $requester } 更改這個應用程式不再詢問，除非要更多權限
chat-card-update-header-added = 這次更新新增的項目：
chat-card-update-header-risky = 這次更新申請了很高或最高級別的權限：
chat-card-update-header-ask-every = 權限沒有放寬，但你開啟了「每次詢問」：
chat-card-update-header-adopt = 這個應用程式是你裝的，同意後交給 { $requester } 管理：
chat-card-update-adopt-replace = 同意後換成它交來的這份，資料保留，裝不上會退回

# 卡片標題。$requester 是 Bot 的名字；$title 是應用程式給的名字。
chat-card-install-title = { $requester } 想安裝 { $title }
chat-card-update-title = { $requester } 想更新 { $title }
chat-card-uninstall-title = { $requester } 想解除安裝 { $title }
chat-card-permission-enable-title = { $requester } 要在「{ $title }」上打開「{ $permission }」
chat-card-permission-disable-title = { $requester } 要在「{ $title }」上關掉「{ $permission }」
chat-card-permission-off-loss = 關掉以後：{ $loss }
chat-card-permission-root = 打開後這個應用程式能控制整台主機，操作以本機 root 身分執行。
chat-choice-permission-enable = 打開
chat-choice-permission-disable = 關掉
chat-permission-done = 已經改好「{ $title }」的權限
chat-permission-failed = 給 { $id } 改權限失敗：{ $error }

# Agent 權限卡片的標題。$title 是 Agent 給這次請求起的名字。
chat-card-agent-title = { $requester }：{ $title }

# 最高級權限要多點一步的那張卡（ADR-0036）。
chat-card-highest-title = { $title } 能控制整台主機
chat-card-highest-line = 程式會以本機 root 身分執行
chat-card-highest-confirm = 我知道這個應用程式能控制整台主機

# 解除安裝卡片：應用程式的資料怎麼處理。
chat-card-uninstall-purge = 並刪除它的資料，與 Bot 的舊加密訊息將無法再查看，無法復原
chat-card-uninstall-keep = 保留它的資料，以後重新安裝還在

# Agent 權限卡片的參數。$args 是原始 JSON，太長時截斷。
chat-card-args = 參數：{ $args }

# Bot 存檔案的卡片。$name 和 $size 描述一個檔案；$total 是加在一起的大小。
chat-card-save-file = { $name }（{ $size }）
chat-card-save-more = 還有 { $count } 個檔案
chat-card-save-total = 共 { $total }
chat-card-save-unreviewed = Bot 提供，內容未審核
chat-card-save-allow = 允許存入 { $folder }
chat-card-save-title = { $requester } 想把 { $count } 個檔案存入 { $folder }

# 給其他 Matrix 用戶端看的卡片內文。$message 是回答卡片時寫的結果行，用你的語言。
chat-card-state-pending = 請在雲吞裡確認。
chat-card-state-running = 正在執行……
chat-card-state-succeeded = 已完成：{ $message }
chat-card-state-failed = 沒有成功：{ $message }
chat-card-state-uncertain = 結果不確定：{ $message }
chat-card-state-rejected = 已拒絕，沒有執行。
chat-card-state-expired = 已過期，沒有執行。

# 卡片內文下面關於放行時段的一行：還在生效，或者怎麼結束的。$count 是放行時段裡除第一次
# 之外放行的次數；0 表示不知道或沒有。
chat-window-active = 放行時段生效中，到期前不會再詢問。
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] 放行時段已到期。
               *[other] 放行時段已到期，放行了 { $count } 次。
            }
        [stopped]
            { $count ->
                [0] 放行時段已停止。
               *[other] 放行時段已停止，放行了 { $count } 次。
            }
        [restarted] 伺服器重新啟動過，放行時段已結束。
       *[other]
            { $count ->
                [0] 放行時段已結束。
               *[other] 放行時段已結束，放行了 { $count } 次。
            }
    }

# 你回答之後寫上卡片的結果行。
chat-install-done = 已裝好 { $title }
chat-install-done-url = 已裝好 { $title }，網址 { $url }
chat-install-failed = 安裝失敗（{ $kind }）：{ $error }
chat-install-failed-id = 安裝 { $id } 失敗（{ $kind }）：{ $error }
chat-install-note-writer = 應用程式的歸屬沒有記下來（{ $error }），下次更新會裝不上
chat-outcome-with-note = { $message }。{ $note }
chat-update-done = 已更新到 { $version }
chat-update-done-url = 已更新到 { $version }，網址 { $url }
chat-update-failed = 更新 { $id } 失敗（{ $kind }）：{ $error }
chat-uninstall-done-purge = 已解除安裝 { $title }，資料已刪除
chat-uninstall-done-keep = 已解除安裝 { $title }，資料保留
chat-uninstall-failed = 解除安裝 { $id } 失敗（{ $kind }）：{ $error }
chat-tool-ran = 已執行
chat-tool-failed = 工具回報執行失敗
chat-save-done = 已存入 { $folder }
chat-save-done-note = 已存入 { $folder }。{ $note }
chat-save-partial = 存好了 { $saved }，後面的沒存成：{ $failure }
chat-save-note-grant = 授權沒有記下來（{ $error }），下次存還要問你
chat-agent-chose = 已選擇「{ $choice }」
chat-agent-abandoned = Agent 不再等這個回答了，這次確認作廢
chat-superseded = 已被新版本取代
chat-uncertain-restart = 伺服器重新啟動過，不確定有沒有執行完

# 踢出理由：Bot 的房間交不到你手裡時，顯示在房間的 timeline 裡。
chat-kick-reason = 雲吞無法完成這個私訊的設定

# 說明工具。Agent 讀了這些再回答你。$details 是上游給的原因；$required 和 $core 是版本號。
chat-help-docs-unavailable = 這台伺服器沒有說明文件（{ $dir } 裡沒有，也沒有取到過）
chat-help-refresh-failed = 取不到新的說明文件：{ $details }
chat-help-refresh-needs-core = 有新的說明文件，但伺服器要更新到 { $required } 以上（這台是 { $core }）：先更新伺服器
chat-help-docs-age = { $problem }。這是 { $days } 天前的文件
chat-help-topic-missing = 沒有主題 { $topic }。可選的主題：{ $topics }
chat-help-page-needs-core = 這一頁講的功能要伺服器更新到 { $required } 以上（這台是 { $core }）：先更新伺服器

chat-mail-convert-failed = 這封信用 HTML 寫成，內文沒能轉成文字，原文已另存為 message.html 附件。
chat-mail-unknown-source = 收到一封來源不明或沒有寄信權限的應用程式郵件，未投遞給任何人。
chat-mail-recipients-failed = 無法更新應用程式郵件的收件人：{ $error }。家人變動或伺服器重新啟動時會再試。
chat-notify-rate-limited = { $plugin } 傳送通知和郵件太頻繁，已暫停，約 { $seconds } 秒後自動恢復。
# Bot 往面板釘頁（ADR-0052）。
panel-pin-card-title = { $bot } 想往你的面板裡放頁面
panel-pin-card-line = 同意後，這個 Bot 以後再往主面板放頁面不再問；可以在設定裡收回。
panel-pin-allowed = 已同意
panel-pin-bot-only = 只有 Bot 能釘頁面；外掛 { $plugin } 沒有 Bot 身分
panel-pin-bot-missing = 找不到 Bot { $bot } 的記錄
panel-pin-full = 面板已經滿了（最多 { $max } 頁），先拿掉一頁再釘
panel-pin-already-asking = 已經有一張往主面板釘頁的確認卡片在等主人回答
panel-pin-matrix = 連不上 Matrix：{ $error }
panel-pin-not-there = 這個頁面沒有釘在面板上

# MCP Apps 界面里的调用在卡片上把调用方写成主人。
mcp-app-owner = 你

# 跳數到頂（ADR-0045）：幾個 Bot 在一個房間裡來回說得太多，閘道停下了。
chat-card-hop-limit-title = { $bots } 來回說了 { $hops } 次，已停下
chat-choice-hop-limit-continue = 繼續
chat-choice-hop-limit-stay = 先停著
chat-hop-limit-resumed = 已繼續
chat-hop-limit-kept-by-other = 另一方選了先停著，房間還停著

# 鏈上限卡片（ADR-0061 第 10 節）：「再給一份」把輪數和花費各加一份預設值，「先停著」停下整條鏈。
chat-chain-limit-title = 這條鏈用到上限了
chat-chain-limit-title-rounds = 這條鏈的輪數用完了
chat-chain-limit-title-cost = 這條鏈的花費用完了
chat-chain-limit-bots = 鏈上的 Bot：{ $bots }
chat-chain-limit-what = 「再給一份」把輪數和花費各加一份預設上限。「先停著」停下整條鏈，要接著做人重新說一句。
chat-chain-limit-grant = 再給一份
chat-chain-limit-stop = 先停著
chat-chain-stopped-card = 鏈被停了，這張卡片作廢

# 傳訊卡片（ADR-0061 第 9 節）：$recipient 是收件人，$bot 是傳訊的 Bot。
chat-send-message-title = 向 { $recipient } 傳送訊息
chat-send-message-queued = 已核准，等 { $bot } 傳送

chat-handoff-title = { $from } → { $bot }
chat-handoff-risk = 這個 Bot 能透過目標執行環境執行命令、改檔案，也能讀出裡面的任何東西，包括登入憑證和別的對話。
chat-handoff-started = 已交接，目標 Bot 正在處理
