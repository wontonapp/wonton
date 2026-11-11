# 外观设置的错误。appearance-settings-unreadable 作为外观页的问题提示（AppearanceProblem.message）显示，其余是它里面的 reason 或 CoreError::Storage 的 reason。
# $path 是设置文件的路径，$reason 是原始的解析或读写错误。
appearance-settings-unreadable = 外观设置读不了，先用默认值：{ $reason }

# $path 是设置文件的路径，$reason 是原始的写入错误。
appearance-settings-write-failed = 写不了外观设置文件 { $path }：{ $reason }
