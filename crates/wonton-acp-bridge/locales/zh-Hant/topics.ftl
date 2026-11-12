## 話題工作階段（ADR-0058）發進聊天和拼給 Agent 的文字。台灣用詞。

name-main = 主對話

topic-interrupt-pending = 忙碌：已有一個打斷請求在等待處理。
topic-no-active-session = 沒有進行中的 ACP 工作階段
topic-steer-rejected = Agent 沒有接受插話請求。
topic-queue-full = ACP 佇列已滿，請稍後再試。
topic-input-too-long = 輸入超過 { $size } 位元組限制
topic-input-empty = 輸入不能為空
topic-control-no-text = 控制命令缺少文字
topic-session-lost = ACP 工作階段已遺失，已建立新的；先前的上下文不再可用。
topic-queued = 話題已排隊：現在有 { $ahead } 個進行中的話題，有話題結束後自動開始。
topic-permission-failed = Agent 權限請求失敗：{ $message }
topic-permissions-cancelled = 權限請求已取消，Agent 沒有執行該操作。
topic-output-limited = 上游錯誤：Agent 回覆超過輸出限制，已取消本輪。
topic-no-text = Agent 本輪沒有產出文字（stopReason={ $reason }）。
topic-idle-ended = 話題閒置超過 30 分鐘，已結束。
topic-main-cannot-end = 主對話不能結束。
topic-already-ended = 這個話題已經結束了。
topic-ended-running = 話題已結束；這一輪還在跑，結果還發進這個話題。
topic-ended = 話題已結束。
topic-ended-idle = 閒置超過 30 分鐘
topic-ended-manual = 使用者手動結束

status-working = 進行中
status-waiting = 等你確認
status-idle = 閒置
status-finished = 已結束
status-queued = 排隊中

conflict-reject-reason = 「{ $other }」正在改 { $path }，使用者選擇不改
conflict-reject-reason-short = 使用者選擇不改
conflict-approved = 「{ $name }」這一輪做完了，已批准改 { $path }。
conflict-in-use-title = { $title }（「{ $other }」也在改）
conflict-wait-option = 等「{ $other }」做完再改
conflict-now-option = 現在就改
conflict-reject-option = 不改
conflict-in-use-note = 「{ $other }」正在改 { $path }；它這一輪做完前改這裡可能被覆蓋。

digest-header = 其他工作階段的動態：
digest-working = 「{ $name }」正在做：{ $doing }
digest-waiting = 「{ $name }」等你確認操作
digest-queued = 「{ $name }」排隊中
digest-intro = 這是從主對話分出來的話題「{ $name }」。
digest-preceding-header = 主對話的片段（僅供參考）：
digest-board-header = 目前工作板：
digest-no-others = 暫無其他工作階段
digest-rules-line = 分工規矩：{ $rules }
digest-rules = 只負責這個話題裡使用者要的事；主對話和其他話題在做的事不要接著做；要碰它們正在改的檔案，先停下問使用者。
digest-first = 使用者的第一句：{ $root }
digest-done = 話題「{ $name }」：已完成
digest-done-files = 話題「{ $name }」：已完成，改了 { $files } 個檔案
digest-done-own = 「{ $name }」已完成
digest-done-summary = 「{ $name }」已完成：{ $summary }
digest-files-count = ，改了 { $files } 個檔案
digest-files-list = ，改了：{ $files }
digest-executed = 「{ $name }」執行了「{ $title }」
digest-conflict = 「{ $first }」和「{ $second }」都改了 { $path }（事後發現）
digest-ended = 「{ $name }」已結束（{ $reason }）
digest-board-line = 「{ $name }」{ $status }
digest-board-line-doing = 「{ $name }」{ $status }：{ $doing }
topic-exec-default = 命令
topic-no-ask-mode = 這個 Agent 沒有「改前先問」的模式（沒設定 WONTON_ACP_TOPIC_MODE 或這個 Agent 不認），改檔案前不會問，只能事後提醒衝突。
