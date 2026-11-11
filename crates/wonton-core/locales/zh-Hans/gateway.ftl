## 网关和核心自己出的网页发给浏览器、App 和 Bot 的文字（ADR-0043）。一条完整的话一条消息，不在代码里拼；
## 数据（主机名、编号、状态、上游给的原因）作参数嵌进句子，不翻。简体中文是现有的说法；对用户叫“应用”，
## 不叫“插件”；不对用户说“核心”，说“家里的服务器”。

# 网关接口、MCP 总线和工具策略共用。
gateway-mcp-disabled = 家里的服务器没有启用 MCP 总线

# 登录页。
gateway-login-title = 登录
gateway-login-heading = 登录 Wonton
gateway-login-token-hint = 用令牌登录（家里服务器的数据目录下的 <code>core/owner-token</code>）；手机上的云吞 App 配对后改为扫码登录。
gateway-login-failed = 令牌不正确。
gateway-login-submit = 登录

# 状态页。
gateway-status-title = 状态
gateway-status-logged-in = 已登录：{ $user }
gateway-status-apps-heading = 应用
gateway-status-empty = 还没有安装应用。
gateway-status-logout = 退出登录

# 状态页上应用的状态。
gateway-state-installing = 安装中
gateway-state-running = 运行中
gateway-state-stopped = 已停止
gateway-state-degraded = 异常：{ $reason }
gateway-state-uninstalling = 卸载中
gateway-state-updating = 升级中

# 错误页。
gateway-page-error-title = 出错了
gateway-page-error-detail = 详情
gateway-not-found = 没有这个页面
gateway-cross-site-write = 家里的服务器只接受自己页面发起的修改请求

# 按 Host 分发和插件子域名上的错误。
gateway-unknown-host = 未知的地址 { $host }
gateway-oidc-unconfigured = 家里的服务器没有配置身份提供方
gateway-no-app-for-host = 没有安装对应 { $host } 的应用
gateway-no-webui = 这个应用没有网页入口
gateway-app-not-ready = 应用 { $id } 尚未就绪
gateway-plugin-not-shared = 应用 { $id } 没有开放给家人
gateway-proxy-sso-failed = 拿不到应用 { $id } 的代登录凭据：{ $error }
gateway-cross-site-login = 网关不认其他网页发来的跨站请求的登录状态，请从应用自己的页面操作
gateway-login-required = 需要先登录 Wonton

# 上游（身份提供方端点、模型网关）出错时显示的名字。
gateway-name-oidc = 身份提供方
gateway-name-models = 模型网关

# SSO 交接和 OIDC 登录、授权确认。
gateway-sso-invalid-next = 跳转地址不合法
gateway-sso-local-only = 只能跳转到本机的插件地址
gateway-sso-missing-code = 缺少交接码
gateway-sso-code-invalid = 交接码无效或已过期，请重新打开
gateway-sso-user-mismatch = 身份提供方记住的是另一个用户
gateway-proxy-sso-member-missing = 你的账号已经不在家里服务器上，要主人重新邀请
gateway-sso-consent-mismatch = 授权请求与当前登录的用户不一致

# 转发给上游。
gateway-upstream-unreachable = { $name } 暂时无法访问：{ $error }
gateway-upstream-timeout = { $name } 迟迟没有回应，可以稍后重试
gateway-invalid-path = 请求路径不合法

# 插件管理接口。
gateway-invalid-body = 请求体不对：{ $error }
gateway-invalid-package = 不是插件包：要 tar、tar.gz、zip 或 compose 文件
gateway-denied = 这台设备没有配对或者已被移除，要重新配对；命令行要带 Authorization: Bearer <令牌>
gateway-cross-site-api = 应用管理接口不接受其他网页发来的请求
gateway-disk-space = 查磁盘空间失败：{ $error }
gateway-invalid-lan-choice = 请求体不是局域网设置的选择：{ $error }
gateway-icon-not-found = 没有这张图片
gateway-icon-failed = 图标处理不了：{ $error }

# 设备和配对码。
gateway-devices-not-found = 没有登记这台设备
gateway-devices-invalid-id = 请求体应为 {"{"}"endpoint_id": "<64 位十六进制>"{"}"}
gateway-devices-invalid-addrs = 请求体应为 {"{"}"direct_addrs": ["IP:端口"]{"}"}：{ $error }
gateway-devices-already-paired = 服务器上已经有配对的设备，不再发首次配对码
gateway-devices-no-addresses = 隧道还没连上中继，请求里也没给直连地址，码里没有能连的地址

# 模型网关。
gateway-models-permission = 应用 { $id } 没有申请 models 权限
gateway-plugin-network-only = 这个地址只给插件网络里的应用用
gateway-models-down = 模型网关没在运行

# 引导。
gateway-onboarding-invalid-step = step 只能是 profile 或 finished，不是 { $step }
gateway-onboarding-invalid-agent = primary_agent 不是可选 Agent：{ $agent }
gateway-onboarding-index-missing = 官方索引没有 onboarding 清单

# 一键升级。
gateway-upgrade-disabled = 家里的服务器没有启用一键升级

# pusher 登记。
gateway-pushers-invalid-body = 注册 pusher 的请求体读不出来：{ $error }
gateway-pushers-invalid-json = 注册 pusher 的请求体不是 JSON：{ $error }
gateway-pushers-proxy-only = pusher 只能指向家里的推送代理 { $url }
gateway-pushers-kind-refused = 不放行 { $kind } 类的 pusher
gateway-pushers-missing-kind = 注册 pusher 的请求体缺少 kind

# MCP 总线和工具策略（按主人的语言写，Agent 和 Bot 读这些）。
gateway-tools-permission = 应用 { $id } 没有申请 tools 权限
gateway-agent-post-only = 这个接口只收 POST
gateway-body-unreadable = 读不了请求体：{ $error }
gateway-body-invalid = 请求体不合法：{ $error }
gateway-bot-not-found = 没有这个 Bot：{ $caller }
gateway-bot-not-in-plugin = 应用 { $id } 没有这个 Bot：{ $caller }
gateway-tool-not-found = 应用 { $id } 没有工具 { $tool }
gateway-plugin-not-running = 这个应用没有在运行，或者没有声明工具

# App 回答确认卡片。
gateway-confirm-invalid-body = 请求体要是 {"{"}"answer": "approve" | "approve_wider" | "allow_window" | "reject"{"}"}：{ $error }
gateway-no-allow-window = 没有生效中的放行时段

# 配对（发给出码扫码的那台手机；按主人的语言写）。
gateway-pair-unknown-code = 服务器没有发过这个配对码
gateway-pair-code-used = 这个配对码已经用过了，请换一个新码
gateway-pair-code-expired = 配对码过期了，请换一个新码
gateway-pair-first-used = 这台服务器已经有配对的设备，首次配对码不能再用；请在服务器上执行 sudo wonton pair 另发一个配对码
gateway-pair-register-failed = 服务器登记设备失败：{ $error }

# 主人在 App 里查看应用的密钥。
gateway-owner-secrets-plugin-not-installed = 应用 { $id } 没有安装
gateway-owner-secrets-record-unreadable = 应用 { $id } 的记录读不出来，列不出它的密钥
gateway-owner-secrets-not-viewable = 应用 { $id } 没有声明可查看的密钥 { $name }
gateway-owner-secrets-not-generated = 密钥 { $name } 还没有值：启动一次应用（或升级）会重新生成
gateway-owner-secrets-store-failed = 读应用密钥失败：{ $error }
gateway-owner-secrets-not-text = 密钥 { $name } 存的值不是文本
gateway-owner-secrets-audit-unavailable = 家里的服务器没有接审计日志，不能查看密钥的值
gateway-plugin-scope-unsupported = 这个核心不支持修改插件给哪些 Bot 用，请先升级核心。
gateway-bot-plugin-access-unsupported = 这个核心不支持为单个 Bot 开关插件，请先升级核心。

# MCP Apps 宿主页（#35）。
mcp-app-loading = 正在载入…
mcp-app-failed = 界面没能载入：{ $error }
mcp-app-page-failed = 界面没能载入。
mcp-app-retry = 重试
