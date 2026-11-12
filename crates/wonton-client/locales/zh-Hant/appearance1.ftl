# 外觀設定的錯誤。appearance-settings-unreadable 作為外觀頁的問題提示（AppearanceProblem.message）顯示，其餘是它裡面的 reason 或 CoreError::Storage 的 reason。
# $path 是設定檔的路徑，$reason 是原始的剖析或讀寫錯誤。
appearance-settings-unreadable = 無法讀取外觀設定，先使用預設值：{ $reason }

# $path 是設定檔的路徑，$reason 是原始的寫入錯誤。
appearance-settings-write-failed = 無法寫入外觀設定檔 { $path }：{ $reason }
