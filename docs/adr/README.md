# 架构决策记录（ADR）

每篇记录一个重大决策：背景、决策、理由、代价与待验证项。

- 推翻旧决策时新写一篇，新篇的状态行写“取代：…”，旧篇的状态行写“…已被 ADR-xxxx 取代”，被取代的段落就地标出或者删掉。
- 状态只写在各篇的状态行，下面的表只列编号和主题。
- 验证结论写进对应 ADR 的“验证结果”一节，太长的单独成文，也列在表里。
- 实施计划（PR 拆分顺序、要同步的文档、要写的测试）放 issue，不写进 ADR。

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
| [0051](0051-friends-and-plugin-sharing.md) | 好友和插件共享：把自己服务器上的插件临时开放给好友 |
| [0052](0052-chat-pull-panel.md) | 聊天页的面板：列表到头后接着拉，拉出自选的页面，两级展开、左右切换 |
| [0053](0053-plugin-dependencies.md) | 插件之间按能力依赖：只写直接依赖，装时一起装，卸时拦下 |
| [0054](0054-bot-command-menu.md) | Bot 的斜杠命令菜单：房间状态事件，整表替换 |
| [0055](0055-model-gateway-optional.md) | 模型网关可选：装 Agent 插件不再要求先有网关 |
| [0057](0057-optional-permissions.md) | 可选权限：声明成默认关，主人自己打开 |
