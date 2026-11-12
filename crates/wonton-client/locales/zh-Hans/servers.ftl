# 多台服务器（crates/wonton-client/src/servers.rs、client/push_route.rs）。
servers-removed = 这台服务器已经从这台手机上移除。

servers-push-from-removed = 发来这条消息的服务器已经从这台手机上移除。

servers-push-scope-closed = 那台服务器已经关掉了。

# $seconds 是等待登记的秒数。
servers-push-register-timeout = { $seconds } 秒内没能登记推送。

# 配过的服务器列表（crates/wonton-client/src/servers.rs）。
# $path 是服务器列表文件的路径，$reason 是原始的解析或读写错误。
servers-list-corrupt = 服务器列表 { $path } 已损坏：{ $reason }

servers-add-current-unpaired = 先配好这一台，再添加别的服务器

servers-name-required = 名字不能为空

# $path 是要读、写或删的路径，$reason 是原始的读写错误。
servers-storage-failed = 读写不了 { $path }：{ $reason }

# 壳把不认识的操作分给服务器列表时显示。
servers-invalid-action = 不是服务器列表操作
