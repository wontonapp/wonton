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

# 本机连接初始化失败；底层错误放在独立诊断中。
home-proxy-start-failed = App 无法启动本地连接代理，请重试。

home-tunnel-closed = 隧道客户端已关闭

home-lan-unsupported = 这个版本的客户端核心不支持局域网设置

home-port-mapping-save-failed = 端口映射的开关存不下：{ $error }

# 连接的本机存储（crates/wonton-client/src/home/storage.rs）。
# $path 是文件的路径，$reason 是原始的解析或读写错误。
home-storage-device-key-corrupt = 设备密钥 { $path } 已损坏：{ $reason }

home-storage-io-failed = 读写不了 { $path }：{ $reason }

# 配对看门时限：$seconds 是整次配对的看门时限，不是交换阶段的限时。
home-pairing-timeout = 配对未能在 { $seconds } 秒内完成。

home-pairing-aborted = 配对没有完成就中断了，再试一次。

home-pairing-not-pairing = 没有正在进行的配对。

# 可读配对详情。$reason 已本地化；elapsed 是实际总耗时，stage 是每个交换阶段的限时，单位均为秒。
home-pairing-failed-detail = { $reason }本次总耗时 { $elapsed_seconds } 秒，配对交换的每个阶段限时 { $stage_seconds } 秒。

home-pairing-joining-pending = 这是加入用的码，正在等管理员同意。
home-pairing-joining-pending-role = 这是加入用的码，正在等管理员同意。同意后加入的角色是“{ $role }”。

# 失败类别不嵌入底层错误原文；原始信息保留在诊断中。
home-failure-transport = 无法建立与家里服务器的连接，或连接已中断。请检查手机和服务器的网络后重试。
home-failure-unauthorized = 家里的服务器拒绝了这台设备，请从已授权的设备或服务器上重新配对。
home-failure-upstream = 已连上家里的服务器，但它需要的服务尚未就绪，请稍后重试。
home-failure-protocol = App 与家里服务器无法按同一协议连接，请检查两端是否需要更新。
home-network-setup-failed = App 无法准备好手机的网络连接，会自动重试。
home-pairing-network-setup-failed = App 无法准备好配对所需的手机网络连接，请重试。
home-proxy-route-failed = App 无法登记本地连接路由，请重试。

# 仅在用户主动打开的可复制诊断中显示；配对密钥和中继令牌会遮去。
home-tunnel-diagnostic = 错误码：{ $code }。原始错误：{ $error }
home-diagnostic-unavailable = 无法安全读取保存的连接信息，因此不能显示诊断信息。
home-pairing-diagnostic = 直连地址：{ $addresses }。中继：[{ $relays }]。{ $error }

# 配对码校验，不包含扫描内容或密钥。
home-code-not-pairing = 这不是云吞配对码，请扫描家里服务器给出的码。
home-code-unsupported-version = 此配对码的格式无法识别，请更新 App 后重试。
home-code-invalid-version = 配对码缺少格式版本，请获取新码后重试。
home-code-invalid-address = 配对码中的服务器连接信息缺失或无效，请获取新码后重试。
home-code-invalid-relay = 配对码中的中继信息无效。中继地址须使用 HTTPS，请获取新码后重试。
home-code-invalid-domain = 配对码中的服务器地址无效，请获取新码后重试。
home-code-invalid-secret = 配对码中的配对密钥缺失或无效，请获取新码后重试。
home-code-invalid = 配对码中的连接信息无效，请获取新码后重试。
home-code-rejected = 服务器没有接受此配对码，请获取新码后重试。
home-code-expired = 此配对码已过期，请获取新码后重试。
home-code-used = 此配对码已被使用或撤销，请获取新码后重试。
home-code-closed = 服务器不再接受这种配对码，请从已授权的设备或服务器添加设备。
home-code-request-invalid = 服务器无法处理配对请求，请检查 App 和服务器是否需要更新。
home-code-server-error = 服务器无法完成配对，请稍后重试。
