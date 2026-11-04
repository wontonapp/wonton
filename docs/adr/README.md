# 架构决策记录（ADR）

每篇记录一个重大决策：背景、决策、理由、代价与待验证项。推翻旧决策时新写一篇，并把旧篇的状态
改为“已被 ADR-xxxx 取代”。M0 的验证结论写进对应 ADR 的“验证结果”一节。

以下决策在 2026-10-28 的设计讨论中确定。

| 编号 | 主题 | 状态 |
| --- | --- | --- |
| [0001](0001-product-positioning.md) | 产品定位、市场与命名 | 已接受 |
| [0002](0002-linux-docker-only.md) | 部署形态：任意 Linux + Docker | 已接受 |
| [0003](0003-matrix-and-encrypted-push.md) | 消息系统：自托管 Matrix + 加密推送中继 | 已接受，M0 验证 |
| [0004](0004-iroh-tunnel.md) | 远程网络：iroh 1.x 应用内隧道 | 已接受，M0 验证 |
| [0005](0005-native-clients-thick-core.md) | 客户端：原生三端 + 厚 Rust 核心 | 已接受，M0 验证 |
| [0006](0006-plugin-system.md) | 插件系统：薄核心 + Compose 插件 | 已接受，M0 选型已定 |
| [0007](0007-agents-and-bots.md) | Agent 与 Bot：Agent 即插件 | 已接受，ACP 桥已验证（#43） |
| [0008](0008-model-gateway-magpie.md) | 模型网关：magpie | 已接受 |
| [0009](0009-identity-pairing-sso.md) | 身份、配对与单点登录 | 已接受，M0 选型已定 |
| [0010](0010-storage-and-nas.md) | 存储与 NAS 能力 | 已接受 |
| [0011](0011-security-model.md) | 安全模型 | 已接受 |
| [0012](0012-updates-release-cloud.md) | 更新、发布与云服务 | 已接受 |
| [0013](0013-engineering-standards.md) | 工程规范 | 已接受 |
| [0014](0014-reserved-capabilities.md) | 预留能力：实时通话、音乐、渠道、Web | 已接受 |
| [0015](0015-small-components-standard-interfaces.md) | 架构原则：小组件与标准接口（UNIX 哲学） | 已接受 |
| [0016](0016-mcp-bus-and-confirmations.md) | MCP 总线与敏感操作的确认 | 已接受 |
| [0017](0017-plugin-ecosystem-and-store.md) | 插件生态：独立仓库、商店和来源 | 已接受，部分被 ADR-0031 取代 |
| [0018](0018-gpu-for-plugins.md) | 显卡交给插件：核心按宿主的显卡给设备 | 已接受 |
| [0019](0019-lan-services-for-plugins.md) | 插件在局域网上提供服务：只发布清单声明的端口 | 已接受 |
| [0020](0020-file-service.md) | 文件服务：Bot 和 App 按编号共用 | 已接受 |
| [0021](0021-third-party-clients.md) | 第三方原生 App 连回家：公网子域名、公共证书和专用密码 | 已接受 |
| [0022](0022-owner-identity.md) | 主人的身份：用户名固定，新插件默认用应用自带的登录 | 已接受 |
| [0023](0023-egress-proxy.md) | 出网插件走宿主的代理：核心注入变量，经出口容器转发 | 已接受 |
| [0024](0024-confirmation-scope.md) | 确认的授权粒度：放宽、撤销和主人亲自操作 | 已接受，判定第 1 步已被 ADR-0027 取代 |
| [0025](0025-core-upgrade.md) | 核心一键升级：升级器容器、状态文件和回滚 | 已接受 |
| [0026](0026-design-system.md) | App 的外观：两端统一自绘、黑白加可换的重点色、Phosphor 图标 | 已接受，部分被 ADR-0029 取代 |
| [0027](0027-bot-save-file.md) | Bot 把文件存进共享文件夹：按路径授权 | 已接受 |
| [0028](0028-settings-page.md) | 设置页：明暗、重点色、字号和字体 | 已接受 |
| [0029](0029-platform-material.md) | App 的材质和形态：令牌共用，栏和菜单按平台走 | 已接受 |
| [0031](0031-plugin-sources.md) | 插件的四种来源：官方、社区、添加的地址和上传的文件 | 已接受 |
| [0032](0032-connection-quality.md) | 和家里服务器的连接：自动恢复，用人话说状态 | 已接受 |
