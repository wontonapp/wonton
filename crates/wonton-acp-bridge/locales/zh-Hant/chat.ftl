## 橋自己送進房間的狀態話（ADR-0043）。一句完整的話一則訊息；資料作參數，不翻。台灣用詞。

login-reconnect = Agent 登入完成，正在重新連線。
input-unsupported = 不支援的輸入：只接受純文字。
input-too-long = 輸入無效：輸入超過 { $max } 位元組上限
input-empty = 輸入無效：輸入不能是空的
control-without-text = 輸入無效：控制命令缺少文字
interrupt-pending = 忙碌：已有一個打斷要求正在等待處理。
queue-full = 忙碌：ACP 佇列已滿，請稍後再試。
no-active-session = Agent 尚未就緒：沒有進行中的 ACP 工作階段
steer-rejected = Agent 沒有接受插話要求。
session-lost-new = ACP 工作階段已遺失，已建立新的工作階段；先前的內容已無法使用。
permission-failed = Agent 權限請求失敗：{ $message }
permissions-cancelled = 權限請求已取消，Agent 沒有執行該操作。
output-limited = 上游錯誤：Agent 回覆超過輸出上限，已取消這一輪。
no-text = Agent 這一輪沒有產出文字（stopReason={ $stop_reason }）。
transport-disconnected = 傳輸無法使用：ACP Agent 連線中斷，正在重新連線。

# 只進日誌（日誌不翻譯，這幾則和類別前綴連在一起讀）。
agent-connect-socket = 連線 ACP socket { $path } 失敗：{ $error }
agent-init-timeout = ACP initialize 逾時
direct-task-failed = 私訊處理工作失敗：{ $error }
stream-closed = Matrix 房間訊息串流已關閉
