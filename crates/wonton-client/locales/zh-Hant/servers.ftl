# 多台伺服器（crates/wonton-client/src/servers.rs、client/push_route.rs）。
servers-removed = 這台伺服器已經從這支手機上移除。

servers-push-from-removed = 傳來這則訊息的伺服器已經從這支手機上移除。

servers-push-scope-closed = 那台伺服器已經關閉了。

# $seconds 是等待登記的秒數。
servers-push-register-timeout = { $seconds } 秒內未能完成推播通知註冊。

# 配過的伺服器清單（crates/wonton-client/src/servers.rs）。
# $path 是伺服器清單檔案的路徑，$reason 是原始的剖析或讀寫錯誤。
servers-list-corrupt = 伺服器清單檔案 { $path } 已損毀：{ $reason }

servers-add-current-unpaired = 先配好這一台，再新增別的伺服器

servers-name-required = 名稱不能為空

# $path 是要讀取、寫入或刪除的路徑，$reason 是原始的讀寫錯誤。
servers-storage-failed = 無法讀寫 { $path }：{ $reason }

# 殼把不認得的操作交給伺服器清單時顯示。
servers-invalid-action = 不是伺服器清單操作
