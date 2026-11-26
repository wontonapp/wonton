# 推送方式设置（crates/wonton-client/src/push_method.rs）。
# $path 是设置文件的路径，$reason 是原始的读写错误。
push-method-write-failed = 写不了推送方式设置文件 { $path }：{ $reason }

# 代理 TUN 的假地址排除出选路的设置（crates/wonton-client/src/home/path_settings.rs）。
# $path 是设置文件的路径，$reason 是原始的读写错误。
path-settings-write-failed = 写不了选路设置文件 { $path }：{ $reason }

# 推送（crates/wonton-client/src/push/{mod,registration,notification}.rs）：被拒绝的地址、失败的登记和没能取回消息的详情。
# 通知的正文和占位（未知的发送者、邀请你加入、[文件]……）还不在这里。
# $reason 是原始错误详情。
push-matrix-pusher-failed = 在 Matrix 上注册 pusher 失败：{ $reason }

# $reason 是 URL 解析器的原始错误。
push-endpoint-invalid-url = 推送地址不是合法的 URL：{ $reason }

push-endpoint-scheme = 推送地址必须是 http 或 https 地址

push-endpoint-p256dh = p256dh 应为 65 字节的未压缩 P-256 公钥（base64url）

push-endpoint-auth = auth 应为 16 字节（base64url）

# $reason 是上游原文，或翻译后的超时详情。
push-register-failed = 登记推送失败：{ $reason }

push-unregister-failed = 取消推送登记失败：{ $reason }

# $reason 是 JSON 解析器的原始错误。
push-register-invalid-response = 核心给的推送登记结果格式不对：{ $reason }

# $reason 是本地代理或 HTTP 客户端的原始错误。
push-invalid-proxy = 本地代理地址不合法：{ $reason }

push-http-client = 建 HTTP 客户端失败：{ $reason }

# $seconds 是请求的超时秒数。
push-timeout = { $seconds } 秒内没有回应

# $status 是原始 HTTP 状态码。
push-server-status = 核心回了 { $status }

# $message 和 $kind 是服务器的原始错误文字和错误码。
push-server-error = { $message }（{ $kind }）

# $reason 是 JSON 解析器的原始错误。
push-payload-invalid = 推送内容的格式不对：{ $reason }

# $reason 是 ID 解析器的原始错误。
push-invalid-room-id = 推送里的房间 ID 不合法：{ $reason }

push-invalid-event-id = 推送里的事件 ID 不合法：{ $reason }

# $seconds 是取消息等的秒数。
push-fetch-timeout = { $seconds } 秒内没能取回这条消息

# $reason 是 Matrix SDK 的原始错误。
push-fetch-failed = 取不到这条消息：{ $reason }

# 附件通知上的文字（ADR-0118）：类型和文件名。
push-preview-image = 图片 { $filename }
push-preview-file = 文件 { $filename }

push-core-closed = 客户端核心已关闭

push-not-paired = 还没有连过家里的服务器

push-home-unreachable = 连不上家里的服务器：{ $reason }

# $seconds 是登录等的秒数。
push-session-timeout = { $seconds } 秒内没能登录 Matrix

push-room-key-missing = 还没收到这条消息的房间密钥

# $path is the private device file; $reason is the underlying storage error.
notification-lock-screen-storage-failed = 读写不了锁屏内容设置 { $path }：{ $reason }

# Privacy page (crates/wonton-client/src/about/mod.rs).
privacy-what-title = 没有云
privacy-what-own = 云吞没有自己的云：服务器就是你家（或你自己租的）那台机器，消息、照片、文件都存在它上面。
privacy-what-none = 我们收不到账号、设备信息，App 里也没有统计、广告和崩溃上报。

privacy-messages-title = 聊天和文件
privacy-messages-e2ee = 聊天和文件都端到端加密，能解开的只有你的设备。
privacy-messages-history = 历史存在你自己的服务器上，备份密钥由你保管。

privacy-push-title = 通知（仅 Android）
privacy-push-keepalive = 用“保持连接”时，通知不走外面，隧道的另一头就是你家。
privacy-push-unified = 改用 UnifiedPush 后，家里的核心先用你设备的公钥把通知加密，再交给分发器；分发器只看得到密文，到手机上才解开。分发器是第三方服务。

privacy-network-title = 怎么连上家里
privacy-network-lan = 同一个局域网时直连；不在一个网络时先打洞，直连不通才经中继转发。
privacy-network-relay = 中继只转发加密后的流量，看不到内容。默认用 iroh 的公共中继，你家可以换成自己的。

privacy-preview-title = 链接预览
privacy-preview-core = 聊天里出现链接时，由你家的核心去取网页的标题和图片，网站看到的是你家服务器的地址。

privacy-plugins-title = 第三方登录和插件网页
privacy-plugins-oidc = 用别的账号登录插件时，账号和密码只发给那个插件的服务器，家里核心不保存第三方密码。
privacy-plugins-web = 插件的网页在你手机的浏览器内核里打开，它能不能出网、出到哪，由插件自己决定。

privacy-device-title = 留在手机上的东西
privacy-device-files = 设备密钥、连接信息、缓存和日志放在 App 的私有目录里，别的 App 读不到；卸载 App 时一起删掉，服务器上的历史不受影响。
privacy-device-backup = 这些文件会跟着系统备份走（iCloud 或 Google 备份），备份存在哪个账号里由系统的设置决定。

# License list load errors (crates/wonton-client/src/about/mod.rs).
licenses-parse-failed = 许可清单解析失败：{ $error }
licenses-empty = 许可清单里一个组件都没有。
licenses-no-text = 许可 { $id } 没有全文。
licenses-none-to-show = 许可清单里没有可显示的许可。
