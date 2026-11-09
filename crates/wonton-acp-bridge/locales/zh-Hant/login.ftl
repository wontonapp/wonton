## Agent 登入流程的訊息（聊天命令、經登入控制 socket 的終端機登入）。命令名不翻。台灣用詞。

login-redact-reason = 已轉交 Agent 登入
login-answer-first = 請先回覆 /wonton login accept 或 /wonton login decline 處理登入連結。
login-code-limits = 授權碼必須是單行文字，且不能超過 16 KiB。
login-not-sent = 登入已結束，這則訊息沒有送給 Agent。
login-no-forward = 登入期間不轉發一般聊天；請在官方頁面完成登入，或傳送 /wonton login cancel。
login-no-direct = 找不到和你的私訊，請在私訊裡傳送 /wonton login。
login-in-direct = 登入在我們的私訊裡進行。
login-cancelled = 登入已取消。
login-no-pending = 沒有等待回答的登入連結。
login-finish-current = 請先結束目前的作業，再發起登入。
login-starting = 正在發起 Agent 登入。取消請傳送 /wonton login cancel。
login-unsupported-method = 目前還不支援這種登入方式。
login-unknown-method = Agent 沒有提供這種登入方式；傳送 /wonton login 查看。
login-secret-delete = 這則訊息裡有登入用的密鑰，請自行刪除。
login-done = Agent 登入完成。
login-no-methods = Agent 沒有提供登入方式。
login-methods-header = 選擇 Agent 登入方式：
login-method-line = { $name }：/wonton login { $id }
login-link =
    Agent 登入：
    { $message }
    { $url }
    打開此連結登入請回覆 /wonton login accept；拒絕請回覆 /wonton login decline。

# 錯誤前綴裡的詳細資訊（見 errors.ftl）。
login-auth-failed = Agent 登入失敗（ACP { $code }）
login-auth-kind-unsupported = 不支援 Agent 提供的登入類型
login-timeout = 登入逾時，請重新發起
login-socket-connect = 連線登入控制 socket { $path } 失敗：{ $error }
login-invalid-frame = 登入 socket 回傳了無效的幀
login-process-exited = Agent 登入程序提前結束
login-failed-detail = Agent 登入失敗：{ $detail }
login-exit-code = Agent 登入程序以結束碼 { $code } 失敗
login-socket-closed = 登入 socket 已中斷：{ $error }
login-chat-closed = 登入聊天已關閉
