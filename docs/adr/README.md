# 架构决策记录（ADR）

每篇记录一个重大决策：背景、决策、理由、代价与待验证项。

- 推翻旧决策时新写一篇，新篇的状态行写“取代：…”，旧篇的状态行写“…已被 ADR-xxxx 取代”，被取代的段落就地标出或者删掉。
- 状态只写在各篇的状态行，下面的表只列编号和主题。
- 验证结论写进对应 ADR 的“验证结果”一节，太长的单独成文，也列在表里。
- 实施计划（PR 拆分顺序、要同步的文档、要写的测试）放 issue，不写进 ADR。

## 新能力检查单

每个新能力（新的插件间能力、新的运行形态、新的外部接口）的 ADR 写清三件：

- 契约（优先开放标准）；
- 至少一个提供方；
- 至少一个真实消费方。

外加一条能证明“换提供方时消费方不用改”的契约测试；样板见
[`model_gateway_contract`](../../crates/wonton-core/src/plugin/tests/model_gateway_contract/)。

| 编号 | 主题 |
| --- | --- |
| [0001](0001-product-positioning.md) | 产品定位、市场与命名 |
| [0002](0002-linux-docker-only.md) | 部署形态：任意 Linux + Docker |
| [0003](0003-matrix-and-encrypted-push.md) | 消息系统：自托管 Matrix + 加密推送中继 |
| [0004](0004-iroh-tunnel.md) | 远程网络：iroh 1.x 应用内隧道 |
| [0005](0005-native-clients-thick-core.md) | 客户端：原生三端 + 厚 Rust 核心 |
| [0006](0006-plugin-system.md) | 插件系统：薄核心 + Compose 插件 |
| [0007](0007-agents-and-bots.md) | Agent 与 Bot：Agent 即插件 |
| [0008](0008-model-gateway-magpie.md) | 模型网关：magpie |
| [0009](0009-identity-pairing-sso.md) | 身份、配对与单点登录 |
| [0010](0010-storage-and-nas.md) | 存储与 NAS 能力 |
| [0011](0011-security-model.md) | 安全模型 |
| [0012](0012-updates-release-cloud.md) | 更新、发布与云服务 |
| [0013](0013-engineering-standards.md) | 工程规范 |
| [0014](0014-reserved-capabilities.md) | 预留能力：实时通话、音乐、渠道、Web |
| [0015](0015-small-components-standard-interfaces.md) | 架构原则：小组件与标准接口（UNIX 哲学） |
| [0016](0016-mcp-bus-and-confirmations.md) | MCP 总线与敏感操作的确认 |
| [0017](0017-plugin-ecosystem-and-store.md) | 插件生态：独立仓库、商店和来源 |
| [0018](0018-gpu-for-plugins.md) | 显卡交给插件：核心按宿主的显卡给设备 |
| [0019](0019-lan-services-for-plugins.md) | 插件在局域网上提供服务：只发布清单声明的端口 |
| [0020](0020-file-service.md) | 文件服务：Bot 和 App 按编号共用 |
| [0021](0021-third-party-clients.md) | 第三方原生 App 连回家：公网子域名、公共证书和专用密码 |
| [0021 的验证](0021-verification.md) | 第三方 App 入口的验证结果和待验证项 |
| [0022](0022-owner-identity.md) | 主人的身份：用户名固定，新插件默认用应用自带的登录 |
| [0023](0023-egress-proxy.md) | 出网插件走宿主的代理：核心注入变量，经出口容器转发 |
| [0024](0024-confirmation-scope.md) | 确认的授权粒度：放宽、撤销和主人亲自操作 |
| [0025](0025-core-upgrade.md) | 核心一键升级：升级器容器、状态文件和回滚 |
| [0026](0026-design-system.md) | App 的外观：两端统一自绘、黑白加可换的重点色、Phosphor 图标 |
| [0027](0027-bot-save-file.md) | Bot 把文件存进共享文件夹：按路径授权 |
| [0028](0028-settings-page.md) | 设置页：明暗、重点色、字号和字体 |
| [0029](0029-platform-material.md) | App 的材质和形态：令牌共用，栏和菜单按平台走 |
| [0030](0030-owner-profile-in-app.md) | 显示名和邮箱在 App 里填 |
| [0031](0031-plugin-sources.md) | 插件的五种来源：官方、社区、添加的仓库、第三方索引和上传的文件 |
| [0032](0032-connection-quality.md) | 和家里服务器的连接：自动恢复，用人话说状态 |
| [0033](0033-plugin-tabs.md) | 底栏的插件页、全屏和 Agent 写插件：网页打底，App 管栏和主题，桥接用 MCP Apps 的词汇 |
| [0034](0034-onboarding-and-help.md) | 新用户引导和帮助文档：选 Agent、接模型、按需求推荐，帮助由核心的工具提供 |
| [0035](0035-acp-bridge.md) | ACP 桥：Matrix 与 ACP Agent 的连接形态 |
| [0036](0036-plugin-privileges.md) | 插件的高权限：声明了就给，按风险分三级提示，由主人决定 |
| [0037](0037-plugin-resource-limits.md) | 插件的资源限制：闲时不限速，忙时让路，内存和进程数有上限 |
| [0038](0038-multi-step-cards.md) | 多步卡片：一组问题一步一张，最后一起提交 |
| [0039](0039-egress-proxy-plugin.md) | 代理插件：没有自己代理的主人，装一个插件就能让出网插件走代理 |
| [0040](0040-registry-mirror.md) | 镜像加速：核心经加速地址按摘要拉 Docker Hub 的镜像 |
| [0041](0041-app-links-and-shortcuts.md) | App 链接、快捷指令和桌面快捷方式 |
| [0042](0042-multiple-servers.md) | 一台手机连多台服务器：同一时间连一台，各自保留数据，每台都推送 |
| [0043](0043-app-languages.md) | App 的多语言：文字放在各平台的本地化资源里，核心只给类别 |
| [0044](0044-bot-profile-and-status-avatars.md) | Bot 的名字、头像和状态头像：资料放在 Matrix，4 个状态由核心和 typing 给出，App 按状态播放 |
| [0045](0045-family-members.md) | 家人和多人：每人一个账号，Bot 有归属人，同机 Bot 经联系人互发消息 |
| [0046](0046-device-verification.md) | 新设备由已有设备确认：登录后用 Matrix 的 SAS 自我验证，和恢复密钥并列 |
| [0047](0047-plugin-notices.md) | 插件通知：插件经核心发进“Wonton”私聊，按级别决定推不推送 |
| [0048](0048-relay-table.md) | 中继表：从配置来，分档自动选优，准入只认 endpoint ID |
| [0049](0049-cloud-hosting.md) | 我们托管的轻量云吞：每用户一台微型虚拟机，里面照旧跑 Docker |
| [0050](0050-cross-server-chat.md) | 跨服务器聊天：配对过的两台服务器经 iroh 互相联邦 |
| [0051](0051-friends-and-plugin-sharing.md) | 好友：云吞是标准的 Matrix 服务器，和整个生态互通，插件按人分享 |
| [0052](0052-chat-pull-panel.md) | 聊天页的面板：列表到头后接着拉，拉出自选的页面，两级展开、左右切换 |
| [0053](0053-plugin-dependencies.md) | 插件之间按能力依赖：只写直接依赖，装时一起装，卸时拦下 |
| [0054](0054-bot-command-menu.md) | Bot 的斜杠命令菜单：房间状态事件，整表替换 |
| [0055](0055-model-gateway-optional.md) | 模型网关可选：装 Agent 插件不再要求先有网关 |
| [0056](0056-internal-mailbox.md) | 每个人一个不变的内部邮箱，应用发的信转进聊天 |
| [0057](0057-optional-permissions.md) | 可选权限：声明成默认关，主人自己打开 |
| [0058](0058-agent-side-sessions.md) | Agent 的话题会话：都能干活，互相知道对方在做什么，改同一个文件时排队 |
| [0059](0059-team-ops-plugins.md) | 多 Agent 团队作战：接入现成的编排器，每个做成一个插件 |
| [0060](0060-chat-layouts.md) | 聊天页的显示方式：气泡或文档，核心合成，壳只画预设 |
| [0061](0061-agent-runtime-and-frontends.md) | Agent 的运行时和前端分开：运行时经一道 ACP 网关提供能力，聊天、网页、看板都是前端 |
| [0062](0062-desktop-vm-host.md) | Mac 和 Windows 当服务器：云吞自带一台隐藏的 Linux 虚拟机 |
| [0063](0063-internal-oidc-issuer.md) | 签发方用插件网络里的内部名字，浏览器走的地址不变 |
| [0064](0064-input-secrets.md) | 用户填写的密钥：清单声明，装时在原生表单里填，核心保管，不回显 |
| [0065](0065-link-previews.md) | 链接预览由家里的核心取，Bot 和插件拿不到；发送时按 MSC4095 附进消息；点链接按核心给的类别打开 |
| [0066](0066-plugin-forms.md) | 插件的几种形态：一条安装管线，按 Bot 生效，目录只给数据 |
| [0067](0067-chat-installing-bot.md) | 装 Agent 插件时聊天列表有一行占位：阶段和类别由核心给，装好由私聊替换 |
| [0070](0070-zero-x-versions.md) | 版本号用 0.x，核心镜像只有 stable 一个渠道 |
| [0071](0071-timeline-position.md) | 聊天时间线的位置：回到底部、新消息数、重进停在哪，都由核心算 |
| [0072](0072-tunnel-udp.md) | 隧道传 UDP：同一条隧道连接上的 QUIC 数据报，插件声明 `tunnel.ports`，App 在本机 `127.0.0.1` 映射 |
| [0073](0073-message-actions-and-replies.md) | 消息菜单和回复：菜单项由核心按消息给，回复、转发、撤回都用 Matrix 的标准做法 |
| [0074](0074-registry-mirrors-per-registry.md) | 镜像加速按仓库分开：ghcr 和 quay 各有自己的名单，一个站不行换下一个 |
| [0075](0075-image-helper-process.md) | 图片处理：常驻助手进程 |
| [0076](0076-device-verification-fallback.md) | 新设备验证：确认方式的取舍，和没有已验证设备时的出路 |
| [0077](0077-plugin-settings.md) | 插件设置：Schema 定义，核心校验，只重建用到的服务 |
| [0079](0079-agent-plugins-monorepo.md) | ACP Agent 插件：一个仓库、一个底座，每家一个子目录 |
| [0080](0080-static-web-plugins.md) | 静态网页插件：文件交给静态托管插件，核心只存文件、转发和记账；外部网页只存一个地址 |
| [0081](0081-local-agent-host.md) | 用户自己电脑上的 Agent：电脑只当远端 ACP 运行时，桥和令牌留在服务器 |
| [0082](0082-agent-run-status.md) | Agent 进度用一条状态消息：按 MSC4564，每轮一条，编辑推进到“等你确认”“完成”“失败” |
| [0085](0085-upgrade-plugin-compat.md) | 核心升级前列出会失效的插件：目标镜像自证的兼容检查 |
| [0088](0088-family-accounts.md) | 家人账号：独立账号、可配置的角色、私人和共享 Bot |
| [0094](0094-chat-attachments.md) | 聊天里附件和文字一起发：附件区预上传，全部传完才发第一条，一组事件带同一个分组标记 |
