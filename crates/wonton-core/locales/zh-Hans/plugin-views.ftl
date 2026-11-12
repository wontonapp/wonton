## 服务器发给 App 的文字（ADR-0043）。一条完整的话一条消息，不在代码里拼；
## 数据（路径、端口、设备名、错误码）作参数嵌进句子，不翻。
## 简体中文是现有的说法；对用户叫“应用”，不叫“插件”。

# 列举数据项（镜像仓库这些）用的分隔符。
list-separator = 、

# 权限：安装确认页、Bot 的确认卡片和商店搜索结果都用它。
permission-storage-share =
    { $access ->
        [read] 读共享文件夹 { $name }
       *[write] 读写共享文件夹 { $name }
    }
permission-storage-home =
    { $access ->
        [read] 读你的私人空间
       *[write] 读写你的私人空间
    }
permission-models = 使用模型网关
permission-tools = 调用家里的服务器和其他应用的工具
permission-plugins-health = 读所有应用的健康状态
permission-matrix-bot = 以 Bot 身份收发消息
permission-notify = 给你发通知
permission-owner-identity = 知道你的用户名和邮箱，还能查家人用什么语言
permission-network-egress = 访问外网和局域网
permission-egress-proxy = 提供出网代理（高权限）
permission-network-lan = 在局域网上开放端口（高权限）
permission-network-host = 使用宿主的网络（很高权限）
permission-capability = 增加内核能力 { $name }
permission-host-pid = 共享宿主进程空间（最高权限）
permission-host-ipc = 共享宿主 IPC（最高权限）
permission-security-opt = 修改容器安全选项（最高权限）
permission-privileged = 以特权模式运行（最高权限）
permission-devices-gpu = 使用 GPU（高权限）
permission-device = 访问宿主设备 { $path }（高权限）
permission-devices-serial = 访问宿主串口设备（高权限）
permission-devices-usb = 访问宿主 USB 设备（高权限）
permission-docker-socket = 控制 Docker（最高权限）

# 高权限下面带的长说明。
permission-notice-egress-proxy = 选它做出网代理后，所有出网应用的请求都经过它：它能看到这些应用访问了哪些网站，也能改写不加密的内容。
permission-notice-network-host = 这个应用能直接访问你家里的网络和本机所有端口，包括只给本机用的服务，也能绕过 Wonton 的登录连到部分其他应用。它开的端口会出现在本机所有网卡上，包括 Wi-Fi、Tailscale 和 IPv6 地址，本机有公网地址时互联网也能连上。本机开着防火墙时，局域网设备连不进来，要你自己放行。
permission-notice-highest-risk = 这个应用能控制整台主机：读写所有文件和其他应用的数据，改动或停掉 Wonton 自己。这等于把主机的 root 交给它。只装你信任的应用。卸载后，它建的容器和改过的东西不会被清理。
permission-notice-network-lan = 局域网里的设备可以直接连接应用开放的端口，不经 Wonton 登录；身份验证由应用自己负责。
permission-notice-device = 这个应用可以访问宿主设备 { $path }。
permission-notice-devices = 这个应用可以访问你交给它的宿主设备。

# 应用详情里的提示。
plugin-firewall-notice = 本机防火墙可能挡住了外来连接，局域网设备可能连不进来
plugin-egress-proxy-select = 在出网代理设置里选它
plugin-cross-origin-isolation = 这个应用放宽了跨源隔离：在 Android 上，它的页面可能和别的网页共用进程。

# 安装和更新预览里的提示与装不了的原因。
install-retained-data = 这台服务器上留着 { $id } 以前的数据和密钥，这个应用装上后能读到它们；不想这样就先彻底删除
install-image = { $service }（{ $image }）
install-images-from = 镜像来自 { $images }
install-images-unpinned = 镜像没有按摘要固定：{ $images }
install-webui-sso = 在 App 里打开它的网页时自动登录
install-host-network-egress = 宿主网络里的服务出网走宿主自己的路由，不经 Wonton 的出网代理
install-system-preinstalled = 系统应用只能随家里的服务器预装
install-already-installed = 已经装了 { $id }（版本 { $version }）
install-unreviewed-provider = 云吞没有审核过这个应用，代码和镜像由 { $provider } 提供
install-unreviewed-provider-via = 云吞没有审核过这个应用，代码和镜像由 { $provider } 提供，经 { $host } 的索引
install-unreviewed-upload-author = 云吞没有审核过这个应用，它来自你上传的文件，清单里写的作者是 { $author }，核实不了
install-unreviewed-upload = 云吞没有审核过这个应用，它来自你上传的文件，清单里没写作者
