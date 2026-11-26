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
permission-models = 用你接好的模型
permission-tools = 用家里服务器和其他应用提供的小工具
permission-plugins-health = 查看所有应用是否正常
permission-matrix-bot = 以聊天机器人身份收发消息
permission-notify = 给家里的人发通知和信
permission-owner-identity = 知道你的用户名和邮箱，还能查家人用什么语言
permission-network-egress = 访问外网和局域网
permission-egress-proxy = 帮其他应用连外网（高权限）
permission-network-lan = 让家里的设备直接连它（高权限）
permission-network-clients = 你打开开关后，让别的应用从家里网络或互联网连进来
permission-network-host = 直接用家里服务器的网络（很高权限）
permission-capability = 增加系统能力 { $name }
permission-host-pid = 能看到服务器上正在运行的所有程序（最高权限）
permission-host-ipc = 和服务器上的程序直接交换数据（最高权限）
permission-security-opt = 改它的安全设置（最高权限）
permission-privileged = 用最大的权限运行（最高权限）
permission-devices-gpu = 用显卡（高权限）
permission-device = 用服务器上的设备 { $path }（高权限）
permission-devices-serial = 用服务器上的串口设备（高权限）
permission-devices-usb = 用服务器上的 USB 设备（高权限）
permission-devices-disk = 读写你勾选的整块硬盘（很高权限）
permission-docker-socket = 管 Docker（最高权限）

# 高权限下面带的长说明。
permission-notice-egress-proxy = 选它当出网代理后，其他应用连外网都要经过它：它能看到这些应用上了哪些网站，也能改掉没加密的内容。
permission-notice-network-host = 这个应用能像服务器自己一样上网，包括只给服务器用的服务，还能绕过云吞的登录去连部分其他应用。它开的端口在所有网络地址上都能连到，家里网络和互联网（服务器有公网地址时）都一样；服务器开着防火墙时，家里的设备连不进来，要你放行。
permission-notice-highest-risk = 这等于把服务器的最高控制权交给它：能读写所有文件和其他应用的数据，能改动或停掉云吞自己。只装你信得过的应用；卸载后它留下的容器和改过的东西不会被清理。
permission-notice-network-lan = 家里的设备能直接连它开的端口，不用登录云吞；来的人让不让进，由它自己定。
permission-notice-device = 这个应用能用服务器上的设备 { $path }。
permission-notice-devices = 这个应用能用你交给它的服务器设备。
permission-notice-devices-disk = 这个应用能读写你勾选的硬盘上的全部数据。
permission-notice-devices-usb = 这个应用能用服务器上所有 USB 设备，包括 U 盘和读卡器。

# 应用详情里的提示。
plugin-firewall-notice = 本机防火墙可能挡住了外来连接，局域网设备可能连不进来
plugin-egress-proxy-select = 在出网代理设置里选它
plugin-cross-origin-isolation = 这个应用要跨源隔离：在 Android 上，它的页面会在浏览器里打开。

# 安装和更新预览里的提示与装不了的原因。
install-retained-data = 服务器上还留着 { $id } 以前的数据和密码，装上后能读到；不想这样就先彻底删掉
install-image = { $service }（{ $image }）
install-images-from = 程序来自 { $images }
install-images-unpinned = 程序没有锁定具体版本：{ $images }
install-webui-sso = 在应用里打开它的网页时自动登录
install-host-network-egress = 它连外网用服务器自己的网络，不经过云吞的代理
install-host-network-proxied = 它连外网也经过云吞的代理
install-already-installed = 已经装了 { $id }（版本 { $version }）
install-unreviewed-provider = 云吞没审核过这个应用，程序由 { $provider } 提供
install-unreviewed-provider-via = 云吞没审核过这个应用，程序由 { $provider } 提供，列在 { $host } 里
install-unreviewed-upload-author = 云吞没审核过这个应用，它来自你上传的文件，清单里写的作者是 { $author }，核实不了
install-unreviewed-upload = 云吞没审核过这个应用，它来自你上传的文件，清单里没写作者
plugin-widget-high = 这个应用的网页能在你打开它的房间里读到聊天内容，或以你的名义发消息、撤回消息。
plugin-widget-normal = 这个应用的网页还能收发通话和给设备的消息、定时发出事件、查通话服务器，并拿到你的登录凭证。
