## 安装、运行应用时给 App 看的文字（ADR-0043）。一条完整的话一条消息，不在代码里拼；
## 数据（路径、端口、镜像名、容器名、Docker 的原话）作参数嵌进句子，不翻。
## 简体中文沿用代码里的说法；对用户叫“应用”，不叫“插件”；不说“核心”，说“服务器”。

# ── 正在进行的操作（manager.rs busy）──────────────────────────────────────────

plugin-install-busy-installing = 应用 { $id } 正在安装
plugin-install-busy-uninstalling = 应用 { $id } 正在卸载
plugin-install-busy-upgrading = 应用 { $id } 正在更新

# ── 安装卸载的冲突和权限（manager.rs、install_entry.rs、written_by.rs）────────

plugin-install-conflict-already-installed = 应用 { $id } 已经装了
plugin-install-conflict-slug = 应用 { $id } 与已安装的 { $other } 使用同一个网页地址
plugin-install-conflict-models-gateway = 已经装了模型网关 { $id }：同一时间只能有一个
plugin-install-conflict-egress-proxy-in-use = 正在使用这个代理插件：先在出网代理设置里换掉，再卸载
plugin-install-conflict-lan-rebuilding = 应用 { $id } 正在按新的局域网地址重建
plugin-install-uninstall-unfinished = 卸载未完成：{ $error }
plugin-install-permission-not-declared = 授予了清单没有声明的权限 { $permission }
plugin-install-permission-unsupported = 服务器还不支持权限 { $permission }
plugin-install-record-unreadable = 应用的安装记录读不懂，卸载（可以保留数据）后重新安装：{ $error }
plugin-install-record-detail-failed = 应用 { $id } 的详情页构造不出来
plugin-install-not-agent-written = { $id } 不是 Agent 写的应用，没有“每次都问”的开关

# ── 回滚（lifecycle.rs）：失败清单的条目和汇总 ──────────────────────────────

plugin-install-rollback-item = { $what }：{ $error }
plugin-install-rollback-unfinished = 回滚未完成，可能有残留：{ $failures }
plugin-install-rb-container = 容器 { $name }
plugin-install-rb-home-room = 交给你的主房间 { $room }
plugin-install-rb-bot-device = Bot { $bot } 的设备
plugin-install-rb-model-credentials = 应用 { $id } 的模型网关凭据
plugin-install-rb-oidc-client = OIDC 客户端 { $id }
plugin-install-rb-volume = 卷 { $volume }
plugin-install-rb-network = 网络 { $network }
plugin-install-rb-core-leave-network = 服务器退出网络 { $network }
plugin-install-rb-egress-leave-network = 出网容器退出网络 { $network }
plugin-install-rb-directory = 目录 { $dir }
plugin-install-rb-secret = 应用 { $id } 的密钥

# name_taken 的词元（lifecycle.rs、leftovers.rs 传进来的是哪类资源）。
plugin-install-word-volume = 卷
plugin-install-word-network = 网络
plugin-install-word-container = 容器

# ── 部署中的失败（lifecycle.rs）─────────────────────────────────────────────

plugin-install-oidc-unconfigured = 服务器没有配置身份提供方（WONTON_OIDC_ADMIN_URL / WONTON_OIDC_PUBLIC_URL）
plugin-install-shared-folder-mkdir-failed = 创建共享文件夹目录 { $dir } 失败：{ $error }
plugin-install-shared-folder-prepare-failed = 准备共享文件夹失败：{ $error }
plugin-install-app-data-prepare-failed = 准备应用数据目录失败：{ $error }
plugin-install-configs-pack-failed = 打包 { $service } 的配置文件失败：{ $error }
plugin-install-teardown-delete-failed = 删除 { $dir } 失败：{ $error }
plugin-install-teardown-leftovers = 卸载后仍有残留：{ $left }

# ── 重名和陌生资源（leftovers.rs）───────────────────────────────────────────

plugin-install-name-taken-owned = { $what } { $name } 属于应用 { $owner }，和本应用的{ $what }重名
plugin-install-name-taken-unknown = { $what } { $name } 已存在，且不是服务器为应用创建的
plugin-install-stranger-attached = 网络 { $network } 上还接着 { $names }，可能是另一套家里的服务器或你自己的容器。一台 Docker 上跑几套家里的服务器时，每套都要设自己的实例名（WONTON_INSTANCE）

# ── 对账和启动（reconcile.rs、start.rs）─────────────────────────────────────

plugin-install-network-mismatch = 应用网络 { $network } { $actual }，按清单和服务器的工作方式应该{ $expected }：重装应用（可以不删数据）后生效
plugin-install-resident-stopped = 应用的常驻服务已停止
plugin-install-resident-partial = 部分常驻服务仍在运行，这些容器已停止：{ $containers }
plugin-install-containers-unhealthy = 部分容器不健康：{ $failures }
plugin-install-container-unhealthy = 容器 { $container } 健康检查失败（最后一次探针，退出码 { $code }）：{ $output }
plugin-install-container-unhealthy-nocode = 容器 { $container } 健康检查失败（最后一次探针，退出码未知）：{ $output }
plugin-install-container-unhealthy-noprobe = 容器 { $container } 健康检查失败（Docker 没留下探针结果）
plugin-install-container-not-running = 容器 { $container } 不在运行（{ $status }）
plugin-install-one-shot-failed = 一次性服务的容器 { $container } 没有以 0 退出（{ $status }，退出码 { $code }）
plugin-install-reach-none = 不存在
plugin-install-reach-egress = 能出网
plugin-install-reach-internal = 不能出网
plugin-install-reach-isolated = 不能出网、和宿主隔开
plugin-install-start-failed = 启动失败：{ $error }
plugin-install-conflict-rebuilding = 应用 { $id } 正在重建，结束后再启动
plugin-install-record-unusable = 应用的安装记录不可用，无法启动
plugin-install-container-missing = 容器 { $name } 不存在
plugin-install-container-not-owned = 容器 { $name } 不属于应用 { $id }

# ── 内存不足（oom.rs）───────────────────────────────────────────────────────

plugin-install-oom-notice = 服务 { $service } 内存不够，被系统结束过{ $limit }
plugin-install-oom-reason = 服务 { $service } 内存不够，被系统结束{ $limit }
plugin-install-oom-limit-both = （服务上限 { $size }，也可能是所有应用加起来到了总上限）
plugin-install-oom-limit-service = （上限 { $size }）
plugin-install-oom-limit-total = （可能是所有应用加起来到了总上限）

# ── 支持的 CPU 和内存上限（limits_update.rs、limits.rs 的接口路径）────────────

plugin-install-limits-busy = 应用 { $id } 正在安装、卸载或重建，等那边做完
plugin-install-limits-min-memory = memory 至少是 { $mib } MiB
plugin-install-limits-ncpu-missing = Docker /info 没有报告 NCPU
plugin-install-limits-ncpu-invalid = Docker /info 报告的 NCPU 不合法
plugin-install-limits-ncpu-out-of-range = Docker /info 的 NCPU 超出范围
plugin-install-limits-cpu-percent-range = cpu_percent 必须在 1 到 { $max } 之间
plugin-install-limits-host-memory-unknown = 读不出主机总内存，恢复默认要等重建
plugin-install-limits-ncpu-bad = Docker 报告的 CPU 核数 { $ncpu } 超出范围
plugin-install-service-error = 服务 { $service }：{ $error }
plugin-install-limits-mem-reservation-order = mem_reservation 不能大于 mem_limit
plugin-install-limits-memory-format = requirements.memory `{ $value }` 不合法
plugin-install-limits-field-forward = { $field }：{ $error }
plugin-install-limits-positive = { $field } 必须大于 0
plugin-install-limits-integer-positive = { $field } 必须是大于 0 的整数
plugin-install-limits-cpus-format = cpus `{ $text }` 不合法
plugin-install-limits-cpus-positive = cpus 必须大于 0
plugin-install-limits-cpus-range = cpus `{ $text }` 超出范围
plugin-install-limits-memory-range = 内存限制超出 Docker 支持的范围

# ── 应用包（package.rs）─────────────────────────────────────────────────────

plugin-install-pkg-yaml = compose 文件不是合法的 YAML：{ $error }
plugin-install-pkg-not-mapping = compose 文件顶层必须是映射
plugin-install-pkg-missing-manifest = compose 文件缺少 `x-wonton` 扩展字段；没有清单的 compose 需要先补全权限声明
plugin-install-pkg-manifest = `x-wonton` 不合法：{ $error }
plugin-install-pkg-service-mapping = 服务定义必须是映射
plugin-install-pkg-unsupported-key = 不支持 `{ $key }`：{ $reason }
plugin-install-pkg-network-mode-unsupported = 不支持 network_mode `{ $mode }`：{ $reason }
plugin-install-pkg-configs-mapping = 必须是映射，写明 file 或 content
plugin-install-pkg-configs-name-chars = 配置名只能包含字母、数字、-、_ 和 .
plugin-install-pkg-configs-key-unsupported = 不支持 `{ $key }`
plugin-install-pkg-configs-file-content-one = file 和 content 只能写一个
plugin-install-pkg-configs-file-content-required = 必须写 file 或 content
plugin-install-pkg-configs-file-content-string = file 和 content 必须是字符串
plugin-install-pkg-mapping-required = 必须是映射
plugin-install-arch-gzip-failed = 解不开 gzip：{ $error }
plugin-install-arch-unpacked-too-big = 解开后超过 { $mib } MiB
plugin-install-arch-too-many-entries = 压缩包里超过 { $max } 项
plugin-install-pkg-top-level = 顶层字段 `{ $key }` 不受支持：{ $reason }
plugin-install-pkg-service = 服务 `{ $service }`：{ $error }
plugin-install-pkg-interpolate = 服务 `{ $service }` 插值失败：{ $error }
plugin-install-pkg-unknown-service = { $field } 引用的服务 `{ $service }` 不存在
plugin-install-pkg-volume = 命名卷 `{ $name }` 不合法：只支持不带参数的本地卷
plugin-install-pkg-config = 配置 `{ $name }`：{ $error }
plugin-install-pkg-config-interpolate = 配置 `{ $name }` 插值失败：{ $error }
plugin-install-pkg-reference = { $field }：{ $error }
plugin-install-pkg-file = { $field } 引用的文件 `{ $path }`：{ $error }
plugin-install-pkg-compose-file = 应用包的 compose.yaml：{ $error }
plugin-install-pkg-archive = 应用包归档：{ $error }
plugin-install-pkg-too-big = 应用包一共 { $kib } KiB，超过 { $max } KiB
plugin-install-pkg-not-utf8 = 不是 UTF-8 文本
plugin-install-pkg-needs-service = 至少需要一个服务
plugin-install-install-icon-file-missing = { $field } { $raw }：文件不存在或不是 PNG/SVG
plugin-install-install-icon-error = 图标 { $path }：{ $reason }
plugin-install-pkg-icon-file = `{ $icon }` 要是应用包里的 PNG 或 SVG 文件
plugin-install-pkg-config-undeclared = 没有在顶层 configs 中声明
plugin-install-pkg-config-missing = 应用包里没有 `{ $path }`
plugin-install-pkg-ref-bot-service = { $field } 不能指向 Bot 服务：Bot 服务每个 Bot 一份
plugin-install-pkg-egress-network-mode = provides.egress_proxy.service 不能写 network_mode
plugin-install-pkg-not-in-subset = 不在服务器支持的 Compose 子集里
plugin-install-pkg-reserved-service = 服务器中继的保留服务名，应用不能使用
plugin-install-pkg-service-name-dns = 服务名必须是合法的 DNS 标签：只含小写字母、数字和 -
plugin-install-pkg-service-name-dashes = 服务名不能以 - 结尾，也不能含 --

# ── 应用包归档（package_archive.rs）─────────────────────────────────────────

plugin-install-arch-read-failed = 读归档失败：{ $error }
plugin-install-arch-path-invalid = 归档里的路径不合法：{ $error }
plugin-install-arch-zip-read-failed = 读 zip 失败：{ $error }
plugin-install-arch-symlink = 软链接
plugin-install-arch-zip-path-invalid = zip 里的路径不合法：{ $error }
plugin-install-arch-two-roots = { $a } 和 { $b } 里都有 compose.yaml：一个压缩包只能放一个应用
plugin-install-arch-missing-compose = 压缩包里没有 compose.yaml
plugin-install-arch-not-regular = `{ $rel }` 不是普通文件（{ $what }）：应用包里不能有软链接、硬链接和设备
plugin-install-arch-duplicate-file = 压缩包里有两个 `{ $rel }`
plugin-install-arch-not-tar = 不是 tar 归档：{ $error }
plugin-install-arch-extract-failed = 解不开 `{ $path }`：{ $error }
plugin-install-arch-read-entry-failed = 读压缩包里的 `{ $path }` 失败：{ $error }

# ── compose 写法（compose.rs）───────────────────────────────────────────────

plugin-install-compose-ports-forbidden = 应用不向宿主暴露端口，网页一律经服务器网关访问
plugin-install-compose-networks-forbidden = 应用网络由服务器创建和管理
plugin-install-compose-container-name-forbidden = 容器名由服务器分配
plugin-install-compose-build-forbidden = 应用只能使用已发布的镜像，不在宿主上构建
plugin-install-compose-env-file-forbidden = 应用包只有一个 compose 文件，环境变量写进 environment
plugin-install-compose-extends-forbidden = 应用包只有一个 compose 文件，不支持 extends
plugin-install-compose-links-forbidden = 同一应用的服务在同一网络内，直接用服务名互访
plugin-install-compose-external-links-forbidden = 应用之间默认互不可达
plugin-install-compose-volumes-from-forbidden = 用命名卷或应用数据目录共享数据
plugin-install-compose-top-networks-forbidden = 应用网络由服务器创建和管理
plugin-install-compose-top-secrets-forbidden = 密钥由服务器注入，compose 里不能声明 secrets
plugin-install-compose-network-host = 宿主网络属于高权限（network.host）
plugin-install-compose-network-mode = network_mode 只接受 none、host 或 service:<本应用的另一个服务>；应用网络由服务器创建和管理
plugin-install-compose-config-external-forbidden = 配置只能来自应用包
plugin-install-compose-config-environment-forbidden = 服务器不把自己的环境变量传给应用，需要变量时用 content 插值
plugin-install-compose-config-name-forbidden = 服务器不创建 Docker 配置对象，不需要 name
plugin-install-compose-pair-missing = { $field } 中的 `{ $key }` 没有值（服务器不传递宿主环境变量）
plugin-install-compose-device-format = 设备映射 `{ $text }` 格式不合法
plugin-install-compose-device-missing-source = 设备映射缺少 source/path_on_host
plugin-install-compose-device-missing-target = 设备映射缺少 target/path_in_container
plugin-install-compose-duration-invalid = 时长 `{ $text }` 不合法（示例：30s、1m30s、500ms）
plugin-install-compose-bytes-invalid = 大小 `{ $text }` 不合法（示例：64m、1g）

# ── configs（configs.rs）───────────────────────────────────────────────────

plugin-install-cfg-undeclared = 配置 `{ $name }` 没有在顶层 configs 中声明
plugin-install-cfg-target-duplicate = configs 的目标 `{ $raw }` 重复
plugin-install-cfg-target-overlaps-mount = configs 的目标 `{ $raw }` 和挂载点 { $mount } 重叠：配置写在容器自己的文件系统里，不能写进挂载点
plugin-install-cfg-target-absolute = configs 的目标 `{ $raw }` 必须是绝对路径，且不能包含 ..
plugin-install-cfg-target-root = configs 的目标不能是 /
plugin-install-cfg-id-invalid = configs 的 { $field } `{ $value }` 必须是非负整数
plugin-install-cfg-mode-invalid = configs 的 mode `{ $value }` 不合法：须为 0 到 0777 的八进制权限

# ── 挂载（mounts.rs）───────────────────────────────────────────────────────

plugin-install-mnt-short-format = 卷 `{ $text }` 格式不合法
plugin-install-mnt-option-unsupported = 卷选项 `{ $bad }` 不受支持
plugin-install-mnt-relative-path = 不支持相对路径 `{ $path }`，使用 { $var } 下的路径
plugin-install-mnt-target-absolute = 挂载目标 { $target } 必须是绝对路径
plugin-install-mnt-volume-undeclared = 命名卷 { $name } 没有在顶层 volumes 中声明
plugin-install-mnt-bind-missing-source = 绑定挂载缺少 source
plugin-install-mnt-host-path-parent = 宿主路径 `{ $path }` 不能包含 ..
plugin-install-mnt-host-path-outside = 宿主路径 `{ $path }` 不在应用数据目录或已授权的共享文件夹里
plugin-install-mnt-shared-folder-whole = 共享文件夹只能整个挂载，不能只挂其中的 `{ $path }`
plugin-install-mnt-shared-folder-readonly = 共享文件夹 { $path } 只授权了读，必须以 :ro 挂载
plugin-install-mnt-nested = 宿主路径 `{ $path }` 在服务 { $outer } 挂载的 { $outer_path } 里面；同一应用的绑定挂载不能嵌套（容器能把内层目录换成软链接，逃出应用数据目录），改用并列的子目录
plugin-install-mnt-owner-conflict = 和服务 { $other } 以不同的属主挂载 { $path }，服务器不知道把它交给谁
plugin-install-mnt-numeric-user = 挂载了应用数据目录，user { $user } 要写成数字形式的 uid:gid：服务器据此决定目录交给谁

# ── 权限（privileges.rs）───────────────────────────────────────────────────

plugin-install-priv-agent-high-risk = Agent 或 Bot 插件不能申请很高或最高级权限 { $permission }
plugin-install-priv-fixed-device = 固定设备权限只支持 /dev/net/tun 和 /dev/fuse，不能申请 { $permission }
plugin-install-priv-host-network-ports = 宿主网络服务不能同时声明 lan.ports；端口由应用直接监听宿主网络
plugin-install-priv-host-network-sso = 宿主网络里的 webui 不能使用 sso: proxy，改用 oidc 或 none
plugin-install-priv-host-network-healthcheck = 宿主网络里的 webui 服务必须写有效的 healthcheck
plugin-install-priv-cap-invalid = cap_add 里的能力 `{ $cap }` 不合法
plugin-install-priv-cap-drop-invalid = cap_drop 里的能力 `{ $cap }` 不合法
plugin-install-priv-security-opt-invalid = security_opt 里的 `{ $opt }` 不合法（no-new-privileges 的值只接受 true 或 false）
plugin-install-priv-device-not-allowed = 设备 { $source } 不是允许的固定设备
plugin-install-priv-device-absolute = 设备目标 { $target } 必须是绝对路径
plugin-install-priv-device-mode = 设备 { $source } 的权限 { $permissions } 不合法
plugin-install-priv-pid-host-only = pid 只支持 host
plugin-install-priv-ipc-host-only = ipc 只支持 host
plugin-install-priv-socket-mount = Docker socket 只能以 /var/run/docker.sock:/var/run/docker.sock 可写挂载
plugin-install-priv-missing-cap-add = 权限 { $permission } 没有对应的 cap_add 字段
plugin-install-priv-missing-devices = 权限 { $permission } 没有对应的 devices 字段
plugin-install-priv-field-requires = { $field } 需要清单权限 { $permission }
plugin-install-priv-missing-field = 清单权限 { $permission } 没有对应的 { $field } 字段
plugin-install-optional-not-allowed = 权限 { $permission } 不能声明成可选
plugin-install-optional-declared-twice = 权限 { $permission } 同时写在 permissions 和 optional_permissions 里
plugin-install-optional-off-loss = 可选权限 { $permission } 要写 off_loss（关着时少了什么），中文和英文都要有
plugin-install-optional-gpu-required = 可选权限 { $permission } 关着时不会给显卡，requirements.gpu.need 不能写 required
plugin-install-optional-not-enabled = 可选项 { $permission } 没有声明成可选
plugin-install-optional-duplicate = 可选项 { $permission } 写了两遍
upgrade-optional-became-required = 新版本把 { $permission } 改成了必需权限，确认后授予
upgrade-optional-removed = 新版本去掉了可选项 { $permission }

# ── provides（provides.rs）─────────────────────────────────────────────────

plugin-install-prv-egress-needs-permission = provides.egress_proxy 必须申请 egress.proxy 权限
plugin-install-prv-permission-needs-egress = egress.proxy 权限必须声明 provides.egress_proxy
plugin-install-prv-egress-needs-network = provides.egress_proxy 必须申请 network.egress 权限
plugin-install-prv-egress-port = provides.egress_proxy.port 不能为 0
plugin-install-prv-empty = provides 至少声明一项能力
plugin-install-prv-models-system-only = 只有 kind: system 的插件能声明 provides.models
plugin-install-prv-models-port = provides.models.port 不能为 0
plugin-install-prv-models-apis-path = provides.models.apis 的路径必须以 / 开头
plugin-install-prv-models-default-model = provides.models.default_model 不能为空
plugin-install-prv-credentials-argv = provides.models.credentials.{ $field } 是非空的 argv，并用 { $placeholder } 指明凭据名
plugin-install-prv-list-argv = provides.models.providers.list 必须是非空的 argv
plugin-install-prv-provider-argv = provides.models.providers.{ $field } 是非空的 argv，并用 { "{{" }id{ "}}" } 指明服务商 ID
plugin-install-prv-custom-argv = provides.models.providers.custom.{ $field } 必须是非空且不含占位符的 argv

# ── 代理插件的 Cookie 和请求头（proxy_auth.rs）──────────────────────────────

plugin-install-proxy-secret-unknown = { "{{" }secret:{ $name }{ "}}" } 要引用 secrets 里由服务器生成（generate）、至少 { $min } 字节的一项
plugin-install-proxy-cookie-template = Cookie 的值只能用 { "{{" }secret:…{ "}}" } 和固定文字：`{ $template }`
plugin-install-proxy-cookie-chars = Cookie 的值里不能有空白、引号、逗号、分号和反斜杠：`{ $template }`
plugin-install-proxy-header-control-chars = 请求头的值里不能有控制字符：`{ $template }`
plugin-install-proxy-header-forbidden = 不能代加请求头 { $name }
plugin-install-proxy-cookie-forbidden = 不能代加 Cookie { $name }
plugin-install-proxy-mcp-prefix = mcp.headers：{ $error }
plugin-install-proxy-mcp-header-forbidden = 不能给 MCP 上游加请求头 { $name }
plugin-install-proxy-mcp-secret-undeclared = { "{{" }secret:{ $name }{ "}}" } 要引用 secrets 里声明的一项
plugin-install-proxy-mcp-identity-placeholder = MCP 头不能用 { "{{" }{ $placeholder }{ "}}" } 这类身份占位符：`{ $template }`
plugin-install-proxy-mcp-header-control-chars = 请求头的值里不能有控制字符：`{ $template }`
plugin-install-proxy-template-open = `{ $template }` 里的 { "}}" } 没有配对的 { "{{" }
plugin-install-proxy-template-close = `{ $template }` 里的 { "{{" } 没有配对的 { "}}" }
plugin-install-proxy-template-unknown-var = `{ $template }` 里的 { "{{" }{ $var }{ "}}" } 不认识：只能用 { "{{" }user{ "}}" }、{ "{{" }name{ "}}" }、{ "{{" }email{ "}}" } 和 { "{{" }secret:<名字>{ "}}" }
plugin-install-proxy-secret-missing = 插件密钥 { $name } 不在密钥库里
plugin-install-proxy-secret-not-text = mcp.headers 的 { $name } 里，密钥 { $secret } 不是文本，放不进请求头
plugin-install-proxy-secret-newline = mcp.headers 的 { $name } 里，密钥 { $secret } 的内容含换行或控制字符（结尾的换行也算）
plugin-install-proxy-secret-no-value = 插件密钥 { $secret } 还没有值，mcp.headers 的 { $name } 引用了它
plugin-install-proxy-mcp-no-identity = mcp.headers 的 { $name } 不能用身份占位符
plugin-install-proxy-mcp-bad-value = mcp.headers 的 { $name } 填出来不是合法的头值
plugin-install-proxy-mcp-bad-name = mcp.headers 的 { $name } 不是合法的头名：{ $error }

# ── 资源要求（requirements.rs、host_check.rs）────────────────────────────────

plugin-install-req-size-format = { $field } `{ $size }` 要写成 512M、2G 这样的大小
plugin-install-req-arch-line = 只支持 { $arches } 架构
plugin-install-req-memory-line = 需要 { $memory } 内存
plugin-install-req-gpu-vram-suffix = ，显存至少 { $memory }
plugin-install-req-gpu-required = 需要{ $card }{ $memory }
plugin-install-req-gpu-optional = 有{ $card }{ $memory }时会用
plugin-install-req-card-generic = 显卡
plugin-install-req-card-named = { $names } 显卡
plugin-install-or-separator = 或
plugin-install-host-unknown = 查不出{ $what }：{ $why }
plugin-install-host-arch-unknown-what = 这台主机的架构
plugin-install-host-arch-mismatch = 这台主机是 { $arch }
plugin-install-host-memory-wanted-what = 要多少内存
plugin-install-host-memory-unknown-what = 这台主机的内存
plugin-install-host-memory-low = 这台主机只有 { $size } 内存
plugin-install-host-gpu-unknown-what = 这台主机的显卡
plugin-install-host-gpu-problem = { $card }：{ $why }
plugin-install-host-gpu-missing = 这台主机没有{ $card }
plugin-install-host-fit-vram-unreadable = 读不懂显存要求 { $text }
plugin-install-host-fit-unknown = 查不出显存：{ $why }
plugin-install-host-fit-integrated = 集成显卡和内存共用，划给它的 { $size } 不是上限，够不够查不出来
plugin-install-host-fit-vram-low = 显存只有 { $size }，要 { $text }

# ── 主机情况（host_facts.rs）───────────────────────────────────────────────

plugin-install-facts-gpu-nvidia = NVIDIA { $device }
plugin-install-facts-gpu-amd = AMD { $device }
plugin-install-facts-gpu-intel = Intel { $device }
plugin-install-facts-gpu-other = 显示设备 { $vendor }:{ $device }
plugin-install-facts-meminfo-missing = /proc/meminfo 里没有 { $field }
plugin-install-facts-meminfo-unreadable = 读不了 /proc/meminfo：{ $error }
plugin-install-facts-docker-not-reported = Docker 没有报告
plugin-install-facts-pci-unreadable = 读不了 { $path }：{ $error }
plugin-install-facts-driver-nouveau = 用的是 nouveau 驱动，不能用于计算，要 NVIDIA 官方驱动
plugin-install-facts-driver-bound = 绑定的是 { $driver } 驱动
plugin-install-facts-driver-none = 没有加载驱动
plugin-install-facts-runtime-missing = Docker 没有登记 nvidia 运行时：要安装 NVIDIA Container Toolkit
plugin-install-facts-runtime-unknown = 不知道 Docker 有没有 nvidia 运行时：{ $reason }
plugin-install-facts-driver-no-render = { $driver } 驱动没有渲染节点，不能用于计算
plugin-install-facts-no-host-dev = 服务器在容器里，但没有挂载宿主的 /dev（WONTON_HOST_DEV），看不到设备节点
plugin-install-facts-dri-missing = 宿主的 /dev/dri 里没有 { $render }
plugin-install-facts-model-unknown = 驱动没有给出型号，只知道设备 ID
plugin-install-facts-vram-not-in-sysfs = NVIDIA 的显存不在 sysfs 里
plugin-install-facts-vram-unknown = 驱动没有给出显存大小
plugin-install-facts-core-not-in-container = 服务器不在容器里，没有镜像可以用来问 NVIDIA 驱动
plugin-install-facts-container-trial-failed = 在容器里试用失败：{ $reason }
plugin-install-facts-container-image-unknown = 查不到服务器容器 { $core } 的镜像
plugin-install-facts-nvidia-smi-exit = nvidia-smi 以 { $code } 退出：{ $reason }

# ── 显卡（gpu.rs）──────────────────────────────────────────────────────────

plugin-install-gpu-needs-permission = 声明了 gpu 就必须申请 devices.gpu 权限
plugin-install-gpu-needs-services = 申请了 devices.gpu 就必须用 gpu.services 写明哪些服务用显卡
plugin-install-gpu-needs-all-three = requirements.gpu 要和 devices.gpu 权限、gpu.services 一起声明
plugin-install-gpu-services-nonempty = gpu.services 至少一项，且不能重复
plugin-install-gpu-describe-vendors = { $vendors } 显卡：{ $devices }，容器加入组 { $groups }
plugin-install-gpu-describe-nvidia = NVIDIA 显卡：全部，经 NVIDIA Container Toolkit
plugin-install-gpu-problem-host-dev = 服务器在容器里，但没有挂载宿主的 /dev（WONTON_HOST_DEV），看不到显卡设备：按当前版本的 compose 重新启动服务器
plugin-install-gpu-problem-unreadable = 读不了 { $path }：{ $error }
plugin-install-gpu-problem-no-device = 没有找到显卡（PCI 上没有显示设备）
plugin-install-gpu-problem-no-driver = 有显卡但没有可用的驱动：{ $detail }
plugin-install-gpu-problem-no-runtime = 有 NVIDIA 驱动，但 Docker 没有登记 nvidia 运行时：要安装 NVIDIA Container Toolkit 并执行 `nvidia-ctk runtime configure --runtime=docker`
plugin-install-gpu-problem-no-wanted-vendor = 没有{ $card }
plugin-install-gpu-vendor-other = 厂商 { $id }
plugin-install-gpu-no-driver-none = 没有加载驱动
plugin-install-gpu-no-driver-nouveau = 用的是 nouveau 驱动，不能用于计算，要 NVIDIA 官方驱动
plugin-install-gpu-no-driver-no-render = 加载了 { $driver } 驱动，但没有渲染节点
plugin-install-gpu-device-entry = { $address }（{ $vendor }）{ $state }
plugin-install-gpu-no-device-nodes = 里面没有设备节点
plugin-install-gpu-preview-granted = 显卡交给服务 { $services }：{ $describe }
plugin-install-gpu-preview-missing = 本机没有符合的显卡（{ $problem }），服务 { $services } 装上后不带显卡运行
plugin-install-gpu-preview-required = 服务 { $services } 需要显卡，{ $reason }

# ── 渲染（render.rs）───────────────────────────────────────────────────────

plugin-install-render-no-owner-identity = 应用没申请 owner.identity，不能引用 `{ $var }`：在清单的 permissions 里加上它
plugin-install-render-bot-dependency = 全应用一份的服务不能依赖 Bot 服务 `{ $service }`：Bot 服务每个 Bot 一份
plugin-install-render-one-shot-restart = 被以 service_completed_successfully 依赖的一次性服务必须写 restart: "no"：否则 Docker 会在它退出后反复重启它
plugin-install-render-network-mode-none = { $field } 指向的服务不能写 network_mode: none
plugin-install-render-dependency-missing = depends_on 引用了不存在的服务 `{ $missing }`
plugin-install-render-dependency-cycle = depends_on 存在循环依赖
plugin-install-render-reserved-label = `wonton.` 开头的标签由服务器保留
plugin-install-render-quotes-unbalanced = 命令 `{ $text }` 的引号不配对
plugin-install-render-restart-invalid = restart 取值 `{ $value }` 不合法
plugin-install-render-healthcheck-format = healthcheck.test 列表须以 CMD、CMD-SHELL 或 NONE 开头

# ── Docker（docker.rs、docker_exec.rs、core_address.rs）──────────────────────

plugin-install-docker-unavailable = 无法连接 Docker：{ $error }
plugin-install-docker-error-join = { $context }：{ $error }
plugin-install-docker-timed-out = Docker 没有按时处理完（{ $context }），操作可能还在继续
plugin-install-docker-not-found = Docker 中不存在：{ $what }
plugin-install-docker-rejected = Docker 拒绝了请求（{ $context }）：{ $error }
plugin-install-port-taken = 端口 { $place } 已被占用，停掉占着它的容器或程序再试。（{ $error }）
plugin-install-port-taken-unknown = 启动容器时端口已被占用，停掉占着它的容器或程序再试。（{ $error }）
plugin-lan-taken-by-plugin = 插件 { $id }
plugin-lan-taken-by-container = 容器 { $name }
plugin-lan-taken-by-host = 宿主上的程序
plugin-install-lan-port-taken = 局域网端口 { $place } 已被{ $who }占着，停掉它再装。
plugin-install-lan-port-hinted = 局域网端口 { $place } 已被{ $who }占着，装到启动那步会失败。
plugin-install-lan-port-probe-failed = 没能检查宿主上的程序是否占着这些端口：{ $reason }
plugin-install-probe-image-replaced = 家里的服务器的镜像已被替换，检测容器用的是它创建时的镜像；重启家里的服务器后恢复（{ $reason }）
plugin-install-probe-image-missing = 家里的服务器的镜像已经不在 Docker 里了，检测容器起不来；重启家里的服务器后恢复。Docker 的原话：{ $error }
plugin-install-docker-pull-failed = 拉取镜像失败（{ $image }）：{ $error }
plugin-install-docker-context-version = 查询版本
plugin-install-docker-context-host-info = 查询 Docker 信息
plugin-install-docker-context-image-healthcheck = 检查镜像 { $image }
plugin-install-docker-context-network = 查询网络 { $name }
plugin-install-docker-no-ipv4-gateway = 网络没有 IPv4 网关
plugin-install-docker-no-subnets = 网络没有地址段
plugin-install-docker-context-remove-network = 删除网络 { $name }
plugin-install-docker-context-list-published-ports = 列出容器已发布的端口
plugin-install-docker-context-connect-network = 把 { $container } 接入网络 { $network }
plugin-install-docker-context-disconnect-network = 把 { $container } 移出网络 { $network }
plugin-install-docker-context-create-volume = 创建卷 { $name }
plugin-install-docker-context-volume-labels = 查询卷 { $name }
plugin-install-docker-context-remove-volume = 删除卷 { $name }
plugin-install-docker-context-create-container = 创建容器 { $name }
plugin-install-docker-context-upload-archive = 往容器 { $container } 写入文件
plugin-install-docker-context-container-missing = 容器 { $container }
plugin-install-docker-context-read-file = 读取容器 { $container } 里的 { $path }
plugin-install-docker-archive-unparseable = 归档无法解析：{ $error }
plugin-install-docker-context-start-container = 启动容器 { $name }
plugin-install-docker-context-inspect-container = 查询容器 { $name }
plugin-install-docker-context-kill-container = 给容器 { $name } 发 { $signal }
plugin-install-docker-context-stop-container = 停止容器 { $name }
plugin-install-docker-context-remove-container = 删除容器 { $name }
plugin-install-docker-context-list-containers = 列出容器
plugin-install-docker-context-list-networks = 列出网络
plugin-install-docker-context-list-volumes = 列出卷
plugin-install-docker-engine-too-old = Docker Engine { $version } 太旧，服务器要求 { $min } 及以上：28 起局域网里的其他主机不能再直接访问容器里没发布的端口，应用网络也才能和宿主完全隔开。请升级 Docker
plugin-install-docker-engine-version-unrecognized = 无法识别 Docker Engine 的版本 `{ $version }`
plugin-install-docker-context-exec = 在容器 { $container } 里执行 { $argv }
plugin-install-docker-exec-no-io = 命令没有挂上输入输出
plugin-install-docker-exec-timed-out = { $context }：{ $seconds } 秒没有结束
plugin-install-coreaddr-no-ipv4-layout = 网络没有能给服务器留地址的 IPv4 网段
plugin-install-coreaddr-pool-exhausted-context = 创建网络 { $name }，服务器给应用网络留的地址段用完了，应用装不上；卸掉不用的应用，或把服务器配置 { $var } 里的地址段改大
plugin-install-coreaddr-pool-exhausted = 地址段 { $subnet }，已用 { $count }/{ $count } 个 /26 网段
plugin-install-coreaddr-kept-taken = { $context }，{ $attempts } 次都和同时建的网络撞了网段
plugin-install-coreaddr-reserved-taken = 预留的网段 { $subnet } 被别的程序刚好占了：重试这次重建，会另挑网段
plugin-install-coreaddr-context-create-network = 创建网络 { $name }
plugin-install-coreaddr-context-inspect-network = 查询网络 { $name }
plugin-install-coreaddr-no-core-address = 网络还没有给服务器留出地址

# ── 上游和局域网（upstreams.rs、host_relay.rs）──────────────────────────────

plugin-install-upstream-container-missing = 容器 { $container } 不存在，或者在网络 { $network } 中没有地址
plugin-install-upstream-mcp-headers = 应用 { $id } 的 MCP 请求头解析不了：{ $error }
plugin-install-relay-prepare-dir-failed = 准备中继目录失败：{ $error }
plugin-install-relay-path-too-long = 应用数据目录的路径太长，unix 套接字放不下，把服务器的数据目录换到短一点的路径（{ $path }：{ $bytes } 字节，上限 { $max } 字节）
plugin-install-relay-listens-unknown = 读不到监听表，不判断防火墙

# ── 图片（images.rs、images/preflight.rs、image_worker.rs）──────────────────

plugin-install-img-tab-icon-alpha = 底栏图标的透明通道不能全不透明或全透明
plugin-install-img-too-many-kib = 超过 { $kib } KiB
plugin-install-img-too-many-mib = 超过 { $mib } MiB
plugin-install-img-not-png-or-jpeg = 不是 PNG 或 JPEG 图片
plugin-install-img-is-format = 是 { $format } 图片，截图只收 PNG 或 JPEG
plugin-install-img-encode-jpeg-failed = 编码 JPEG 失败：{ $error }
plugin-install-img-not-supported-format = 不是支持的图片格式
plugin-install-img-format-no-thumbnail = { $format } 做不了缩略图
plugin-install-img-over-edge-or-memory = 超过 { $max }×{ $max } 像素或内存上限
plugin-install-img-decode-failed = 解码失败：{ $error }
plugin-install-img-png-too-big = PNG 图标超过 { $kib } KiB
plugin-install-img-png-content = 文件名是 .png，内容却不是 PNG
plugin-install-img-not-square = 图标要是正方形，这张是 { $width }×{ $height }
plugin-install-img-png-too-small = PNG 图标至少 { $min }×{ $min }，这张是 { $width }×{ $height }
plugin-install-img-svg-too-big = SVG 图标超过 { $kib } KiB
plugin-install-img-svg-not-utf8 = SVG 要是 UTF-8 文本（不收压缩过的 SVG）
plugin-install-img-svg-invalid = 不是合法的 SVG：{ $error }
plugin-install-img-svg-parse-failed = 解析 SVG 失败：{ $error }
plugin-install-img-svg-viewbox = SVG 的 viewBox 要是正方形，这张是 { $width }×{ $height }
plugin-install-img-canvas-alloc-failed = 分配画布失败
plugin-install-img-canvas-convert-failed = 转换画布失败
plugin-install-img-over-max-edge = 超过 { $max }×{ $max } 像素
plugin-install-img-encode-png-failed = 编码 PNG 失败：{ $error }
plugin-install-img-svg-root = 根元素要是 <svg xmlns="{ $ns }">
plugin-install-img-style-rules = 样式表最多 { $max } 条规则，这里有 { $count } 条
plugin-install-img-style-combinator = 样式表的选择器 “{ $selector }” 带了组合符：只能用 .cls-1 这样不带组合符的选择器
plugin-install-img-nesting-depth = SVG 嵌套超过 { $max } 层
plugin-install-img-use-cycle = SVG 里的 <use> 循环引用
plugin-install-img-use-expanded = SVG 展开 <use> 后超过 { $max } 个元素
plugin-install-img-use-href-count = <use> 要有且只有一个 href（href 或 xlink:href）
plugin-install-img-use-href-format = <use> 的 href 只能写成 #id，id 里不能有空白：{ $href }
plugin-install-img-element-forbidden = 图标不能用 <{ $name }>
plugin-install-img-clip-path-units = <clipPath> 的 clipPathUnits 只能是 userSpaceOnUse
plugin-install-img-mask-units = <mask> 要写 maskUnits="userSpaceOnUse"，maskContentUnits 只能是 userSpaceOnUse
plugin-install-img-gradient-stops = 渐变最多 { $max } 个 <stop>，这个有 { $count } 个
plugin-install-img-worker-usage = 用法：wonton-core { $command } <icon.png|icon.svg|tab-icon.png|tab-icon.svg|screenshot|thumbnail|preview>（服务器内部用）
plugin-install-img-worker-unavailable = 处理图片的进程不可用：{ $error }
plugin-install-img-worker-rlimit = 设不了资源上限：{ $error }
plugin-install-img-worker-stdin = 读标准输入失败：{ $error }
plugin-install-img-worker-stdout = 写标准输出失败：{ $error }
plugin-install-img-worker-spawn-failed = 启动 { $program } 失败：{ $error }
plugin-install-img-worker-communicate = 和子进程通信失败：{ $error }
plugin-install-img-worker-timeout = 处理超过 { $seconds } 秒，已停止
plugin-install-img-worker-crashed = 处理时崩溃（退出码 { $code }）
plugin-install-img-worker-cpu-timeout = 处理超过 { $seconds } 秒 CPU 时间，已停止
plugin-install-img-worker-memory-limit = 处理时要的内存超过 { $mib } MiB
plugin-install-img-worker-killed = 处理图片的进程被杀掉了（SIGKILL），多半是内存不够
plugin-install-img-worker-signal = 处理图片的进程被信号 { $signal } 结束
plugin-install-img-worker-malformed = 处理图片的进程输出的格式不对

## 预装（启动时随服务器带的包）失败的原因。
plugin-install-preinstall-retrying = 预装失败（已试 { $attempts } 次，稍后自动重试）
plugin-install-preinstall-failed = 预装失败

# ── 能力依赖（capabilities.rs，ADR-0053）──────────────────────────────────

plugin-capability-unknown = 这份核心不认识能力 { $capability }，升级核心后再装
plugin-capability-third-party-format = 能力 { $capability } 不合法：第三方能力要写成 x-<作者>.<名字>，作者和名字用小写字母、数字、连字符
plugin-capability-egress-no-consumer = egress_proxy 没有使用方声明，不能写在 requires 或 wants 里
plugin-capability-duplicate = { $field } 里的能力 { $capability } 写了两次
plugin-capability-in-both-lists = 能力 { $capability } 不能同时在 requires 和 wants 里
plugin-capability-provider-requires = 提供能力的插件不能 requires 别的能力
plugin-capability-port-zero = provides.{ $capability }.port 不能为 0
plugin-capability-credentials-required = provides.{ $capability } 要声明 credentials：给每个使用方签一个只属于它的凭据
plugin-capability-credentials-forbidden = provides.{ $capability } 不能声明 credentials：这个能力的提供方只认经核心转发来的请求
plugin-capability-credentials-argv = provides.{ $capability }.credentials.{ $field } 是非空的 argv，并用 { $placeholder } 指明凭据名
plugin-capability-service-unknown = { $field } 引用的服务 `{ $service }` 在 compose 里不存在
plugin-capability-service-not-forwardable = { $field } 不能指向 network_mode: none 的服务：核心要转发进去、在里面签发凭据
plugin-install-capability-deps-unsupported = 这个核心还不支持插件之间的依赖，升级核心后再装
plugin-install-runtime-services = runtime 为 { $runtime } 时不能声明 services
plugin-install-runtime-unsupported = 这个核心不支持安装或升级 runtime 为 { $runtime } 的插件，请先升级核心
