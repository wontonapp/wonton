# $error 是 JSON 解析器的原始错误。
home-ticket-invalid-json = 连接信息不是合法的 JSON：{ $error }

# 没有参数。扫到的连接信息里没有能连上服务器的地址。
home-ticket-no-addresses = 连接信息里没有服务端的中继地址或直连地址

home-ticket-relays-invalid = 中继表不合法：{ $reason }

# $domain 是连接信息里的原始基础域名。
home-ticket-invalid-domain = 基础域名不合法：{ $domain }

# $error 是地址解析器的原始错误。
home-ticket-invalid-web-url = 插件网页地址不合法：{ $error }

# $domain 是连接信息的基础域名，$url 是连接信息里的原始地址。
home-ticket-web-url-under-domain = 插件网页地址要是 { $domain } 下的 http(s) 地址：{ $url }

# $error 是地址解析器的原始错误。
home-ticket-invalid-core-url = 核心地址不合法：{ $error }

# $domain 是连接信息的基础域名，$url 是连接信息里的原始地址。
home-ticket-core-url-under-domain = 核心地址要是 { $domain } 的 http(s) 地址：{ $url }

# $error 是网络初始化的原始错误。
# 没有参数。配对页的设备名一栏被清空了。
home-pairing-name-empty = 设备名是空的。

home-pairing-bind-failed = 绑定网络失败：{ $error }

# $seconds 是绑定的时限秒数。建连时本机网络在这么久里没准备好，会自动再试。
home-bind-timeout = 本机网络在 { $seconds } 秒内没有准备好。

# $reason 是连接信息解析器的原始错误。
home-ticket-corrupt = 保存的连接信息已损坏：{ $reason }

home-no-connection-info = 还没有家里服务器的连接信息

# $error 是本地代理原始的绑定错误。
home-proxy-start-failed = 本地代理启动失败：{ $error }

home-tunnel-closed = 隧道客户端已关闭

home-lan-unsupported = 这个版本的客户端核心不支持局域网设置

home-port-mapping-save-failed = 端口映射的开关存不下：{ $error }

# 连接的本机存储（crates/wonton-client/src/home/storage.rs）。
# $path 是文件的路径，$reason 是原始的解析或读写错误。
home-storage-device-key-corrupt = 设备密钥 { $path } 已损坏：{ $reason }

home-storage-io-failed = 读写不了 { $path }：{ $reason }

# 配对详情。地址和中继 URL 不含配对密钥；error 保留上游原文。
home-pairing-timeout = 配对未能在 { $seconds } 秒内完成。

home-pairing-aborted = 配对没有完成就中断了，再试一次。

home-pairing-not-pairing = 没有正在进行的配对。

home-pairing-failed-detail = 直连地址：{ $addresses }。中继：[{ $relays }]。已用时 { $milliseconds } 毫秒，限时 { $seconds } 秒。原因：{ $error }

home-pairing-joining-pending = 这是加入用的码，正在等管理员同意。
home-pairing-joining-pending-role = 这是加入用的码，正在等管理员同意。同意后加入的角色是“{ $role }”。
home-pairing-joining-rejected = 管理员拒绝了这个加入请求。
home-pairing-joining-code-used = 这个加入用的码已经被别的设备用过了。
