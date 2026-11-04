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
| [0031](0031-plugin-sources.md) | 插件的五种来源：官方、社区、添加的仓库、第三方索引和上传的文件 |
| [0032](0032-connection-quality.md) | 和家里服务器的连接：自动恢复，用人话说状态 |
| [0033](0033-plugin-tabs.md) | 底栏的插件页、全屏和 Agent 写插件：网页打底，App 管栏和主题，桥接用 MCP Apps 的词汇 |
| [0034](0034-onboarding-and-help.md) | 新用户引导和帮助文档：选 Agent、接模型、按需求推荐，帮助由核心的工具提供 |
