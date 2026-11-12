## 商店、来源、网络设置、模型服务商和应用升级里给 App 看的文字（ADR-0043）。
## 一条完整的话一条消息，不在代码里拼；数据（路径、端口、镜像名、ID、地址、Docker 的原话）
## 作参数嵌进句子，不翻。对用户叫“应用”，不叫“插件”；不说“核心”，说“服务器”。

# ── 通用 ─────────────────────────────────────────────────────────────────────

## with_note 把原消息和补充说明拼成一句。en 用半角括号加空格。
plugin-settings-note-appended = { $message }（{ $note }）

# ── 网页的局域网端口的风险说明（lan/webui.rs，App 详情页显示）────────────────

plugin-settings-lan-webui-risk-impersonate = 局域网里的设备和这台主机上能出网的应用，不用密码就能以你的身份打开它
plugin-settings-lan-webui-risk-register = 管理员注册之前，局域网里的设备和这台主机上能出网的应用可能抢先注册
plugin-settings-lan-webui-risk-client = 局域网里的设备拿到账号和密码就能连上它。在它自己的网页里改了密码，这里不会跟着变

# ── 清单的 lan 和 lan.client（lan/mod.rs、lan/client.rs）─────────────────────

plugin-settings-lan-client-prefix = lan.client：{ $message }
plugin-settings-lan-client-address-host = address 要写 { $host }
plugin-settings-lan-client-template = 不能用 { $name }
plugin-settings-lan-need-ports = 申请了 network.lan 就必须用 lan.ports 写明开放哪些端口
plugin-settings-lan-need-permission = 声明了 lan 就必须申请 network.lan 权限
plugin-settings-lan-no-matrix-bot = network.lan 不能和 matrix.bot 一起申请：地址变了要重建容器，服务器不保存 Bot 的访问令牌
plugin-settings-lan-ports-empty = lan.ports 至少一项
plugin-settings-lan-port-zero = lan.ports 的端口不能为 0
plugin-settings-lan-port-duplicate = lan.ports 里的 { $port } 重复了
plugin-settings-lan-port-mcp = mcp 的端口只经服务器访问，不能列进 lan.ports（{ $service } 的 { $port }）
plugin-settings-lan-network-none = lan.ports 里的服务不能写 network_mode: none

# ── lan.client 的连接信息（lan/client.rs，详情页给主人看）────────────────────

plugin-settings-lan-client-secret-missing = 密钥库里没有这个应用的记录
plugin-settings-lan-client-container = 查不到容器 { $container }：{ $error }
plugin-settings-lan-client-not-published = 端口没有开在任何局域网地址上

# ── 局域网检测和选择（lan/detect.rs、lan/select.rs、lan/monitor.rs）──────────

plugin-settings-lan-fixed-by-env = 局域网地址由服务器的 WONTON_LAN_ADDRESSES 指定，不能在设置里改
plugin-settings-lan-announce-by-env = 手机连的地址由服务器的 WONTON_ANNOUNCE_ADDRESSES 指定，不能在设置里改
plugin-settings-lan-desktop-vm = 服务器跑在电脑上的虚拟机里，手机连不到虚拟机的地址。启动器要把 WONTON_ANNOUNCE_ADDRESSES 设为电脑的局域网地址。内核：{ $evidence }
plugin-settings-lan-announced = 宣告
plugin-settings-lan-fixed-missing = WONTON_LAN_ADDRESSES 里的 { $ip } 不在主机上
plugin-settings-lan-detect-failed = 检测主机的网卡失败：{ $failure }
plugin-settings-lan-detect-error = 检测主机的网卡时出错：{ $error }
plugin-settings-lan-detect-core-image = 查不到服务器容器 { $container } 的镜像
plugin-settings-lan-detect-exit = 检测容器以 { $code } 退出：{ $reason }
plugin-settings-lan-detect-output = 检测容器的输出不对：{ $error }
plugin-settings-lan-detect-timeout = 容器 { $container } { $seconds } 秒没跑完
plugin-settings-lan-detect-no-exit-code = Docker 没给出容器 { $container } 的退出码
plugin-settings-lan-listen-error = 检查端口占用时出错：{ $error }
plugin-settings-lan-choice-unknown = 存下的局域网设置这个版本的服务器不认识，要重新选
plugin-settings-lan-no-default-route = 主机没有 IPv4 默认路由
plugin-settings-lan-default-virtual = 默认路由走的是 { $name }（{ $why }），不自动选
plugin-settings-lan-default-no-private = 默认路由所在的网卡 { $name } 上没有私有 IPv4 地址
plugin-settings-lan-interface-gone = 选的网卡 { $name } 不在主机上了，或者上面没有私有地址
plugin-settings-lan-interface-no-private = 选的网卡 { $name } 上没有私有 IPv4 地址
plugin-settings-lan-address-gone = 选的地址 { $address } 不在主机上了
plugin-settings-lan-no-selection = 一个网卡或地址都没选
plugin-settings-lan-choice-unrecognized = 不认识这种选法
plugin-settings-lan-interface-not-selectable = { $name } 不是主机上可以选的网卡
plugin-settings-lan-address-not-selectable = { $address } 不是主机上可以选的地址
plugin-settings-lan-kind-tunnel = 隧道网卡，如 Tailscale
plugin-settings-lan-kind-wireguard = WireGuard 网卡
plugin-settings-lan-kind-interface = 网卡
plugin-settings-lan-bridge-or-veth = Docker 的网桥或者 veth
plugin-settings-lan-bound = { $interface } 的 { $ip }
plugin-settings-lan-select-failed = 选不出局域网地址：{ $reason }
plugin-settings-lan-lookup-failed = 查不到局域网端口发布在哪里：{ $error }
plugin-settings-lan-rebuild-failed = 按新的局域网地址重建失败：{ $error }
plugin-settings-lan-check-failed = 核对应用失败：{ $error }

# ── 安装预览里的局域网端口（lan/grant.rs）────────────────────────────────────

plugin-settings-lan-port-line = 局域网端口：{ $service } 的 { $key }，开在 { $place } 上
plugin-settings-lan-port-line-plain = 局域网端口：{ $service } 的 { $key }
plugin-settings-lan-direct-notice = 局域网里的设备直接连这些端口，不经服务器登录，身份由应用自己验证
plugin-settings-lan-conflict-taken = 局域网端口 { $key } 已被应用 { $id } 占用
plugin-settings-lan-note-reselect = 可以在局域网设置里改选网卡或地址

# ── 网页的局域网端口开关（lan/webui.rs）──────────────────────────────────────

plugin-settings-lan-webui-no-switch = 应用 { $id } 没有 lan_webui 开关：它的局域网端口不用开关
plugin-settings-lan-webui-busy = 应用 { $id } 正在安装、卸载或重建，等那边做完
plugin-settings-lan-webui-close-failed = 关网页的局域网端口时失败：{ $error }
plugin-settings-lan-webui-rollback-failed = 改开关失败后退回也失败：{ $error }

# ── 安装错误的整句（install_error.rs 各变体的构造点）────────────────────────

plugin-settings-not-installed = 应用 { $id } 没有安装
plugin-settings-permission-not-granted = 以下权限需要你确认后才能安装：{ $permissions }
plugin-settings-core-upgrading = 家里的服务器正在更新，更新结束后再试
plugin-settings-bot-matrix-unavailable = 服务器没有接入 Matrix（WONTON_MATRIX_URL），声明 matrix.bot 的应用装不了
plugin-settings-note-revoke-device-failed = 吊销安装设备失败：{ $error }
plugin-settings-note-migration-containers-gone = 迁移网络时旧容器已经删除：重试这次重建，或重装应用（可以不删数据）恢复
plugin-settings-note-registry-mirror = 可以在设置里打开镜像加速
plugin-settings-upgrade-rolled-back = 更新到 { $new } 失败，已退回 { $version }
plugin-settings-upgrade-rollback-failed = 更新到 { $new } 失败，退回 { $version } 也失败：{ $error }
plugin-settings-upgrade-cleanup-unfinished = 收拾未完成，可能有残留：{ $failures }
plugin-settings-upgrade-cleanup-model-credentials = 模型网关凭据：{ $error }

# ── 出网代理（egress/）───────────────────────────────────────────────────────

plugin-settings-egress-plugin-and-url = 用代理插件时不能同时填代理地址
plugin-settings-egress-restarts-changed = 要重启的应用和确认时不一样了：重新确认
plugin-settings-egress-cidr-invalid = 网段 { $cidr } 不合法：{ $error }
plugin-settings-egress-cidr-overlaps-network = 网段 { $cidr } 和 Docker 网络 { $network }（{ $subnet }）重叠：放行的话出口容器会直接拨到别的应用
plugin-settings-egress-env-locked = 代理地址由 WONTON_EGRESS_PROXY 指定，不能在这里改：改安装目录的设置后再执行一次安装脚本
plugin-settings-egress-address-invalid = 代理地址不合法：{ $error }
plugin-settings-egress-address-scheme = 不支持代理协议 { $scheme }：只能用 http、https 或 socks5
plugin-settings-egress-address-no-host = 代理地址里没有主机名
plugin-settings-egress-address-no-path = 代理地址只写协议、主机和端口，不带路径和参数
plugin-settings-egress-address-utf8 = 代理地址里的用户名或密码不是合法的 UTF-8：{ $error }
plugin-settings-egress-no-response = 代理 { $address } { $seconds } 秒没响应：宿主防火墙要放行 Docker 网段到这个端口
plugin-settings-egress-refused = 代理 { $address } 拒绝连接：端口没人听，或者只监听回环地址
plugin-settings-egress-unreachable = 连不上代理 { $address }：{ $error }
plugin-settings-egress-upstream-failed = 代理本身是通的，它连不上外面：{ $error }
plugin-settings-egress-rejected = 代理这一步没通过：{ $error }
plugin-settings-egress-disconnected = 代理 { $address } 断开了连接：{ $error }
plugin-settings-egress-not-http = 代理 { $address } 回的不是 HTTP
plugin-settings-egress-auth-407 = 代理 { $address } 回 407：要用户名和密码，或者写错了
plugin-settings-egress-status = 代理 { $address } 回 { $code }
plugin-settings-egress-relay-not-running = 出网容器没有在运行
plugin-settings-egress-relay-detail = 直接连代理是通的，经出网容器不通：{ $detail }。查看容器 { $container } 的日志
plugin-settings-egress-diagnose-exit = 查原因的容器以 { $code } 退出：{ $reason }
plugin-settings-egress-provider-missing = 没有装能做出网代理的应用
plugin-settings-egress-provider-manifest = 代理插件 { $id } 的声明读不出来
plugin-settings-egress-provider-conflict = 已经装了代理插件 { $id }：同一时间只能有一个
plugin-settings-egress-provider-not-running = 代理插件没在运行：{ $detail }
plugin-settings-egress-relay-cidr-overlap = 网段 { $cidr } 和出网容器接入的 Docker 网络重叠，没有放行：放行的话它会直接拨到别的应用
plugin-settings-egress-token-unreadable = 服务 { $service } 把 Matrix 令牌写在 command 或 configs 里，读不回来
plugin-settings-egress-token-reinstall = 重装应用（可以不删数据）后会签发新设备，Bot 要重新建加密身份
plugin-settings-egress-token-read-back = Bot 的令牌读不回来：{ $reason }。{ $reinstall }

plugin-settings-egress-bot-record-missing = 没有这个应用的 Bot 记录
plugin-settings-egress-old-container-gone = 旧容器 { $container } 不在了
plugin-settings-egress-token-not-in-env = 旧容器 { $container } 的环境变量 { $variable } 里没有令牌
plugin-settings-egress-token-mismatch = 旧容器里的令牌不一致
plugin-settings-egress-token-not-used = 旧版本没有服务用到令牌

# ── 镜像加速（registry_mirror/）──────────────────────────────────────────────

plugin-settings-mirror-address-invalid = 镜像加速地址 { $address } 不合法：只填主机名和可选端口，不带协议、路径或用户名密码
plugin-settings-mirror-policy-never = 镜像 { $image } 标了 pull_policy: never，本地又没有
plugin-settings-mirror-all-failed = 镜像 { $image } 从哪个加速地址都拉不下来；{ $tries }。请去设置里测速、换一个加速地址
plugin-settings-mirror-try-failed = { $mirror }：{ $error }
plugin-settings-mirror-digest-mismatch = 本地镜像的 RepoDigests 没有要求的摘要 { $digest }，拒绝使用
plugin-settings-mirror-probe-timeout = 测速总超时（{ $ms } 毫秒）：{ $error }
plugin-settings-mirror-no-challenge = 缺少有效的 Bearer WWW-Authenticate
plugin-settings-mirror-token-request-rejected = 匿名令牌请求被拒绝
plugin-settings-mirror-token-response-invalid = 匿名令牌响应无效：{ $error }
plugin-settings-mirror-token-missing = 匿名令牌响应缺少 token/access_token
plugin-settings-mirror-manifest-rejected = 清单请求被拒绝
plugin-settings-mirror-manifest-digest = 清单摘要不匹配：期望 { $digest }，收到 { $actual }
plugin-settings-mirror-status = HTTP { $status }：{ $detail }

# ── 应用商店和来源（storefront/、store.rs、store_info.rs）────────────────────

plugin-settings-store-source-unreadable = 添加的来源读不了：{ $why }
plugin-settings-store-source-not-added = 没有添加过 { $address }
plugin-settings-store-source-limit = 最多添加 { $max } 个来源，先删掉不用的
plugin-settings-store-source-save-failed = 添加的来源存不了：{ $error }
plugin-settings-store-source-in-official = { $repo } 已经在官方商店里了
plugin-settings-store-source-neither = { $repo } 的根目录没有 { $compose }，也没有 { $index }：不是应用，也不是索引
plugin-settings-store-source-already-added = { $address } 已经添加过了
plugin-settings-store-source-index-missing = { $provider } 没有索引文件
plugin-settings-store-source-not-an-index = { $address } 不是索引：{ $error }
plugin-settings-store-source-plugin-missing = 索引 { $provider } 里没有 { $id }
plugin-settings-store-community-partial = 有的没读到：{ $missing }
plugin-settings-store-community-page-failed = 搜索结果的第 { $page } 页（{ $error }）
plugin-settings-store-community-unavailable = 取不到社区应用：{ $error }
plugin-settings-store-system-app = 系统应用只能随服务器预装
plugin-settings-store-permission-unsupported = 服务器还不支持权限 { $permission }
plugin-settings-store-index-newer = 索引的格式是 { $format }，这个版本的服务器只读得懂 { $supported }：更新服务器后才能用新的索引
plugin-settings-store-index-invalid = 索引读不懂：{ $error }
plugin-settings-store-index-compose-digest = compose 和 files 里 { $file } 的 sha256 对不上
plugin-settings-store-index-version-mismatch = version { $version } 和清单的版本 { $manifest } 不一致
plugin-settings-store-index-source-duplicate = source 重复
plugin-settings-store-index-id-duplicate = 插件 ID { $id } 重复
plugin-settings-store-index-source-with-version = source 不能带版本
plugin-settings-store-index-commit-length = commit 要是 40 位提交号
plugin-settings-store-index-file-digest = { $repo } 的 { $path } 和索引里记的 sha256 对不上
plugin-settings-store-index-file-missing = 索引里 { $source } 的文件没有 { $path }
plugin-settings-store-index-blob-invalid-sha = 官方索引里的 sha256 不合法：{ $sha256 }
plugin-settings-store-index-blob-missing = 索引目录里没有 { $path }：{ $error }
plugin-settings-store-record-id-mismatch = 记录里的 ID 是 { $record }，与文件名 { $file } 不符

# ── 商店清单的资料（store_info.rs，装官方应用时给 App 的原因）────────────────

plugin-settings-store-info-spec-missing = 清单要写 spec，如 spec: "{ $version }"
plugin-settings-store-info-spec-string = spec 必须是字符串，如 "{ $version }"（YAML 里要加引号，不然读成数字）
plugin-settings-store-info-spec-version = spec { $text } 不是规范版本（如 "{ $version }"）
plugin-settings-store-info-author = author.name 不能为空，不超过 { $max } 个字符
plugin-settings-store-info-license = license { $license } 不是 SPDX 许可证表达式（如 MIT、AGPL-3.0-only）：{ $error }
plugin-settings-store-info-screenshots-count = screenshots 要有 1 到 { $max } 张
plugin-settings-store-info-screenshot-format = 截图 { $raw } 要是 PNG 或 JPEG
plugin-settings-store-info-screenshot-duplicate = 截图 { $raw } 重复了
plugin-settings-store-info-tags-count = tags 最多 { $max } 个
plugin-settings-store-info-tag-length = 标签 { $tag } 要有 1 到 { $max } 个字符，前后不带空白
plugin-settings-store-info-tag-control = 标签 { $tag } 里有控制字符
plugin-settings-store-info-tag-duplicate = 标签 { $tag } 重复了
plugin-settings-store-info-field-length = { $field }.{ $lang } 超过 { $max } 个字符
plugin-settings-store-info-https-url = { $field } { $value } 要是 https:// 开头的网址

# ── 更新检查（upgrade/）────────────────────────────────────────────────────

plugin-settings-upgrade-none = 应用 { $id } 没有新版本
plugin-settings-upgrade-no-origin = 应用 { $id } 没有来源，查不到新版本
plugin-settings-upgrade-id-changed = 插件包是 { $package }，要更新的是 { $record }：更新不能改 id
plugin-settings-upgrade-same-version = 和装着的 { $version } 一样，不用换
plugin-settings-upgrade-changed = { $id } 的新版本在预览之后变了（插件包摘要现在是 { $digest }），要重新预览、重新确认
plugin-settings-upgrade-older = 比装着的 { $version } 旧。应用的数据可能已经按新版本迁移过，旧版本可能读不了，先备份
plugin-settings-upgrade-source-changed = 来源从 { $old } 换成 { $new }，换了之后云吞不再按原来的来源提示更新
plugin-settings-upgrade-origin-upload = 你上传的文件
plugin-settings-upgrade-origin-index = { $via } 的索引
plugin-settings-upgrade-index-unavailable = 官方索引没取到，查不了新版本：{ $details }
plugin-settings-upgrade-system-app = 系统应用随服务器更新，接口更新不了
plugin-settings-upgrade-lan-ports-closing = 局域网端口 { $ports } 更新后会关掉，要继续开着就打开下面的开关
plugin-settings-upgrade-check-failed = { $id } 的新版本没查到：{ $error }
plugin-settings-upgrade-official-commit-moved = 官方索引里版本 { $version } 换了提交：不算新版本，装着的没变
plugin-settings-upgrade-tag-moved = 这个版本的 tag { $tag } 被作者改到了别的提交：不算新版本，装着的没变
plugin-settings-upgrade-index-removed = 索引 { $provider } 已经从商店删掉，不再提示新版本
plugin-settings-upgrade-compose-not-utf8 = { $file } 不是 UTF-8 文本
plugin-settings-upgrade-manifest-id-changed = 清单的 id 变成了 { $id }
plugin-settings-upgrade-manifest-version = 清单的版本是 { $version }
plugin-settings-upgrade-bundled-failed = 随服务器带来的 { $version } 没升成，还在跑 { $from }，下次启动再试：{ $error }

# ── 访问 GitHub（github.rs、github_source.rs）───────────────────────────────

plugin-settings-github-tags-what = 读 { $repo } 的 tag
plugin-settings-github-no-repo = GitHub 上没有公开仓库 { $repo }
plugin-settings-github-status = { $what } 时 GitHub 回了 { $status }
plugin-settings-github-tags-parse = GitHub 给的 { $repo } 的 tag 读不懂：{ $error }
plugin-settings-github-file-what = 取 { $repo } 在提交 { $commit } 里的 { $path }
plugin-settings-github-missing = 没有 { $what }
plugin-settings-github-url-what = 取 { $url }
plugin-settings-github-api-what = 经 API 取 { $repo } 在 { $reference } 里的 { $path }
plugin-settings-github-rate-limited = { $what }：GitHub 限速了，几分钟后再试
plugin-settings-github-rate-limited-plain = GitHub 限速了，几分钟后再试
plugin-settings-github-api-status = { $what }：GitHub 回了 { $status }
plugin-settings-github-search-url = 搜索的地址不对：{ $error }
plugin-settings-github-search-what = 搜索社区应用
plugin-settings-github-search-status = 搜索社区应用时 GitHub 回了 { $status }
plugin-settings-github-search-parse = GitHub 给的搜索结果读不懂：{ $error }
plugin-settings-github-context = { $what }：{ $error }
plugin-settings-github-timeout = { $what }：请求超时，没有在时限内答完
plugin-settings-github-unreachable = { $what }：连不上 GitHub（{ $error }）
plugin-settings-github-too-big = 超过 { $max } KiB
plugin-settings-github-read-what = 读响应
plugin-settings-github-no-tag = { $repo } 没有 tag { $tag }
plugin-settings-github-no-release = { $repo } 没有正式版本：要有 v<语义化版本> 的 tag，如 v1.0.0
plugin-settings-github-no-release-tag = 没有 v<语义化版本> 的正式版本 tag
plugin-settings-github-tag-version-mismatch = tag { $tag } 和清单的版本 { $version } 不一致：发布时 tag 要是 v<清单的版本>
plugin-settings-github-tag-version-short = tag { $tag } 和清单的版本 { $version } 不一致
plugin-settings-github-source-invalid = 来源 `{ $text }` 不对：{ $why }；写成 github:<owner>/<repo>[/<子目录>]，可以带 @<tag> 或 @<40 位提交号>
plugin-settings-github-source-scheme = 只支持 github: 开头的来源
plugin-settings-github-source-no-name = 缺少仓库名
plugin-settings-github-source-name-chars = owner 和仓库名只能有字母、数字、`-`、`_`、`.`
plugin-settings-github-source-subdir-empty = 子目录里有空的段；开头或结尾的 `/` 也算空段
plugin-settings-github-source-subdir-long = 子目录超过 200 个字符
plugin-settings-github-source-subdir-dot = 子目录里有 `.` 或 `..` 段
plugin-settings-github-source-subdir-chars = 子目录只能有字母、数字、`-`、`_`、`.`、`/`
plugin-settings-github-source-bad-tag = @ 后面的 tag 不合法
plugin-settings-github-sibling-url = { $url } 旁边的 { $path } 不是合法地址：{ $error }

# ── 来源和索引的地址（source_address.rs、index_location.rs）─────────────────

plugin-settings-source-address-invalid = 地址 `{ $text }` 不对：{ $why }
plugin-settings-source-address-tls = 只支持 https：未审核的来源要靠 TLS 确认内容没被改过
plugin-settings-source-address-https-only = 只支持 https 地址和 GitHub 仓库
plugin-settings-source-address-formats = 写成仓库地址、owner/repo，或者索引的 https 地址
plugin-settings-source-not-a-repo = `{ $text }` 不是应用仓库。索引要在商店里添加，再从那里装
plugin-settings-source-page-invalid = GitHub 地址 `{ $path }` 不对：写成 https://github.com/<owner>/<repo>，可以带 /tree/<tag>[/<子目录>] 或 /commit/<提交>
plugin-settings-index-address-invalid = 索引地址 `{ $text }` 不对：{ $why }
plugin-settings-index-address-https = 只支持 https
plugin-settings-item-source-invalid = 来源 `{ $source }` 不对

# ── 模型网关和模型服务商（models.rs、model_providers.rs）────────────────────

plugin-settings-models-gateway-not-running = 模型网关 { $id } 没在运行
plugin-settings-models-gateway-absent = 没有装模型网关（声明 provides.models 的系统应用）
plugin-settings-models-revoke-not-running = 模型网关没在运行，吊销不了 { $id } 的凭据
plugin-settings-models-revoke-provider-not-running = 模型网关 { $provider } 没在运行，吊销不了 { $id } 的凭据
plugin-settings-models-reconcile-failed = 模型网关凭据对账失败：{ $error }
plugin-settings-models-credential-rejected = 模型网关 { $provider } 不认服务器签发的凭据 { $name }：签发后请求模型列表仍然 401
plugin-settings-models-unreachable = 模型网关 { $provider } 不可达：{ $error }
plugin-settings-models-list-error = 模型网关 { $provider } 回答模型列表时出错：HTTP { $status }
plugin-settings-models-provider-error = 默认模型的服务商出错：{ $error }
plugin-settings-models-provider-invalid-reply = 服务商没有返回合法的对话结果
plugin-settings-models-list-malformed = 模型网关 { $provider } 的模型列表格式不合法
plugin-settings-models-connect-first = 先在“模型设置”接入模型，Bot 才能回话。
plugin-settings-models-not-ready = 模型网关 { $provider } 未就绪：{ $error }
plugin-settings-models-command-rejected = 模型网关 { $provider } 拒绝了凭据命令（{ $command }，退出码 { $code }）：{ $stderr }
plugin-settings-model-action-save = 保存
plugin-settings-model-action-test = 测试
plugin-settings-model-disconnected-saved = 服务商连接失败，key 已保存
plugin-settings-model-disconnected-unsaved = 服务商连接失败，什么都没保存
plugin-settings-model-no-provider-commands = 模型网关 { $provider } 没有声明服务商命令
plugin-settings-model-provider-unsupported = 模型网关不支持服务商 { $id }
plugin-settings-model-custom-unsupported = 模型网关不支持自定义服务商
plugin-settings-model-custom-encode-failed = 自定义服务商请求编码失败
plugin-settings-model-command-timeout = 模型网关 { $provider } 命令执行失败或超时
plugin-settings-model-invalid-endpoint = 服务商地址或模型名不合法
plugin-settings-model-key-rejected = 服务商拒绝了 key，key 没有保存
plugin-settings-model-provider-command-failed = 模型网关 { $provider } { $action }服务商失败，退出码 { $code }
plugin-settings-model-list-failed = 模型网关 { $provider } 列出服务商失败：{ $error }
plugin-settings-model-list-exit = 模型网关 { $provider } 列出服务商失败，退出码 { $code }
plugin-settings-model-list-not-json = 模型网关 { $provider } 的服务商列表不是合法 JSON
plugin-settings-model-key-invalid = key 不能为空、不能有换行，且不能超过 4 KiB
plugin-settings-model-url-invalid = 服务商地址必须是 http 或 https URL、不能有空白，且不能超过 2 KiB
plugin-settings-model-name-invalid = 模型名不能为空、不能有空白或控制字符，且不能超过 200 字节
plugin-settings-model-request-format = 模型服务商请求格式不正确

# ── 商店索引（storefront）──────────────────────────────────────────────────

plugin-settings-store-index-fetch-failed = 取不到官方索引：{ $error }

# ── 健康检查（health.rs）───────────────────────────────────────────────────

plugin-settings-health-timeout = 容器 { $container } 在期限内没有就绪（最后状态：{ $state }）
plugin-settings-health-unhealthy = 容器 { $container } 健康检查失败
plugin-settings-health-exited = 容器 { $container } 已退出（退出码 { $code }）
plugin-settings-health-no-healthcheck = 容器 { $container } 被要求 service_healthy，但没有配置健康检查
plugin-settings-health-webui-timeout = 网页入口 { $url } 在期限内没有响应：{ $last_error }
plugin-settings-health-log-tail = 最近日志：
plugin-settings-health-probes = 最近探针：
plugin-settings-health-probe-line = [退出码 { $code }] { $output }
plugin-settings-health-probe-line-unknown = [退出码 ?] { $output }
plugin-settings-health-container-missing = 查不到容器 { $container }
plugin-settings-health-one-shot-exit = 一次性服务的容器 { $container } 没有以 0 退出（{ $status }，退出码 { $exit_code }）
plugin-settings-health-not-running = 容器 { $container } 不在运行（{ $status }）

# ── 插件密钥（secrets.rs）──────────────────────────────────────────────────

plugin-settings-secret-missing = 应用的密钥 { $name } 丢失
plugin-settings-secret-not-text = 应用的密钥 { $name } 不是文本
plugin-settings-secret-invalid = 应用的密钥 { $name }：容器 { $container } 里的 { $path } { $reason }
plugin-settings-secret-not-regular = 不是普通文件
plugin-settings-secret-too-big = 超过 64 KiB

# ── 插件的 Bot（bots.rs）───────────────────────────────────────────────────

plugin-settings-bot-avatar-missing = bot.avatar 指向的 { $path } 不在插件包里
plugin-settings-bot-avatar-type = bot.avatar 只支持 png、jpg、gif、webp：{ $path }
plugin-settings-bot-device-name = Wonton 应用 { $id }（{ $bot }）
plugin-settings-bot-account-deactivated = Bot 账号 { $user } 已停用：选择删除数据卸载这个应用后重装，会分配新账号
plugin-settings-bot-device-record-missing = 没有 Bot { $bot } 的设备记录：重装应用（可以不删数据）
plugin-settings-bot-device-revoked = Bot { $user } 的设备已被吊销，应用拿着的 Matrix 令牌失效了：重装应用（可以不删数据）重新签发

# ── 局域网检测、出网容器和商店索引的补充（lan/detect.rs、docker_limits.rs、egress、storefront）──

plugin-settings-lan-detect-wait-exit = 等一次性容器退出
plugin-settings-lan-detect-read-output = 读一次性容器的输出
plugin-settings-lan-detect-list-networks = 列出网络
plugin-settings-limits-update-container = 更新容器 { $name }
plugin-settings-mirror-docker-proxy = 查询 Docker 代理
plugin-settings-mirror-inspect = 检查镜像
plugin-settings-mirror-inspect-image = 检查镜像 { $image }
plugin-settings-mirror-pull-image = 拉取镜像 { $image }
plugin-settings-egress-rebuild-failed = 按新的代理设置重建失败：{ $error }
plugin-settings-egress-gost-config = 生成 gost 的配置失败：{ $error }
plugin-settings-egress-prepare-dir = 准备 { $dir } 失败：{ $error }
plugin-settings-egress-write-file = 写 { $path } 失败：{ $error }
plugin-settings-egress-remove-dir = 删除 { $dir } 失败：{ $error }
plugin-settings-store-index-no-format = 没有 format
plugin-settings-store-index-docs-problem = 官方索引的 docs 读不懂：{ $error }
plugin-settings-store-index-onboarding-problem = 官方索引的 onboarding 读不懂：{ $error }
plugin-settings-store-index-mirrors-problem = 官方索引的 registry_mirrors 读不懂：{ $error }
plugin-settings-store-index-read-failed = 读不了官方索引 { $path }：{ $error }

plugin-secret-unknown = 请求里有不认识的密钥：{ $names }
plugin-secret-required-missing = 必填的密钥还没填：{ $names }
plugin-secret-required-clear = 必填的密钥 { $name } 不能清空
plugin-secret-not-input = { $name } 不是主人填写的密钥
chat-card-secrets-not-used = 这张确认不收密钥

# ── 插件设置的值和 Schema（ADR-0077）────────────────────────────────────────

plugin-settings-schema-invalid-json = 设置文件不是合法 JSON
plugin-settings-schema-object = 设置 Schema 必须是对象 Schema
plugin-settings-schema-properties = 设置 Schema 缺少 properties 对象
plugin-settings-schema-too-many = 设置 Schema 的项超过 64 个
plugin-settings-schema-draft = $schema 必须是 https://json-schema.org/draft/2020-12/schema
plugin-settings-schema-additional = additionalProperties 必须是 false
plugin-settings-schema-unsupported = 原生控件不支持 { $key }
plugin-settings-schema-required-duplicate = required 里的 { $key } 重复了
plugin-settings-schema-required-unknown = required 里的 { $key } 不是属性
plugin-settings-schema-required = required 必须是属性名列表
plugin-settings-schema-order = x-wonton-order 里每个属性最多出现一次
plugin-settings-schema-field = 每个属性必须是 Schema 对象
plugin-settings-schema-name = 属性名以小写字母开头，只用小写字母、数字和 _，最长 64 个字符
plugin-settings-schema-secret-name = 这个名字也是密钥名，改掉其中一个
plugin-settings-schema-sensitive = 敏感项，挪到输入密钥
plugin-settings-schema-title = 缺少 x-wonton-title
plugin-settings-schema-description = x-wonton-description 必须按语言键写
plugin-settings-schema-type = type 必须是单个受支持的类型
plugin-settings-schema-enum = enum 至少要有一个不重复的字符串
plugin-settings-schema-pattern = pattern 不是合法正则
plugin-settings-schema-bound = minimum 和 maximum 必须是数字
plugin-settings-schema-length = minLength 和 maxLength 必须是非负整数
plugin-settings-schema-items = 数组的 items 必须是字符串 enum
plugin-settings-schema-unique = 数组项必须写 uniqueItems: true
plugin-settings-schema-enum-duplicate = enum 里有重复的值
plugin-settings-schema-enum-titles = x-wonton-enum-titles 必须给每个枚举值一个标题，不多不少
plugin-settings-schema-problems = 设置 Schema 有 { $count } 处不合格
plugin-settings-value-unknown-field = 这个应用没有这一项
plugin-settings-value-missing = 必填，填一下
plugin-settings-value-wrong-type = 必须是 { $type }
plugin-settings-value-below-minimum = 不能小于 { $min }
plugin-settings-value-above-maximum = 不能大于 { $max }
plugin-settings-value-too-short = 至少 { $min } 个字符
plugin-settings-value-too-long = 最多 { $max } 个字符
plugin-settings-value-pattern = 不符合要求的格式
plugin-settings-value-duplicate = 有重复的值
plugin-settings-value-not-in-options = 从选项里挑一个
plugin-settings-value-string-too-big = 超过 { $max } 字节
plugin-settings-value-array-too-big = 超过 { $max } 项
plugin-settings-value-too-big = 设置合计超过 { $max } 字节
plugin-settings-value-unknown-type = 这个 App 版本发不了这种值，更新 App
plugin-settings-value-problems = { $count } 项设置不合格
