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
