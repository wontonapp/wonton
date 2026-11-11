# ADR-0012 更新、发布与云服务

- 状态：已接受（2026-10-28）；渠道一句已被 ADR-0070 取代

## 背景

旧链路用约 4.6 万行 shell/mjs 自建签名 OTA、installer 快照和多 mirror 事务提交，manifest 过期、
bot 读不到包等发布事故反复出现；发布要走 test、canary、prod 多个分支。

## 决策

- **核心升级**：核心镜像只有 stable 一个渠道（ADR-0070 取代了原来的 stable、beta 两个渠道），新装的主机默认走它（ADR-0025）。App 里一键升级：核心拉起一次性的升级器容器，
  拉新镜像、重建核心容器、做健康检查，失败就回滚到旧镜像，具体做法见 ADR-0025。安装的目录布局和升级时要改的地方见
  `deploy/install/README.md`。
- **插件升级**：按官方索引里的版本或插件仓库的新 tag 提示，主人确认后升级（ADR-0017）。以后可以给“官方插件、权限
  没变”的升级打开自动。
- **镜像分发**：同时发布到 GHCR 和国内镜像仓库，安装脚本按网络自动选择；以镜像 digest 作为版本身份。
  先只发 GHCR（`ghcr.io/wontonapp/wonton-core`）；国内拉 ghcr.io 可能慢，国内镜像仓库
  以后按这一条加上。安装脚本放在单独的公开小仓库：免费账号下，私有仓库的 Release 附件和 Pages 都不能匿名下载。
- **我们运营的云服务**只有五类：iroh 中继与发现、推送中继、节点列表（签名静态文件）、国内镜像、第三方 App 的
  入口（中继、家名服务、acme-dns、权威 DNS，ADR-0021）。没有用户账号，也不存业务数据。第三方 App 的入口存的是
  运行它必需的数据：`<家>` 和注销后的墓碑表、endpoint ID、局域网地址、CAA 账号 URI、TCP 端口分配表、令牌的哈希、
  邀请码、用量和连接日志（保留期见 ADR-0021），流量只按 SNI 或端口转发，不解密。插件的官方索引在 GitHub 上（ADR-0017），以后签了名也经节点列表那样的静态文件分发。
- **节点**：部署安排见 [GOAL](../GOAL.md) 的 M2。第三方 App 入口的默认地址写在安装目录的 `.env` 里，不经节点列表。
- **App 分发**：MVP 走 TestFlight 和直装 APK；应用商店上架在 M3。
- **CI**：GitHub Actions，发布由 tag 触发。作业全部跑在 GitHub 托管 runner 上（仓库公开后 Linux 和 macOS
  分钟都免费，macOS 不再按 10 倍计分钟）：lint 在 ubuntu-24.04，Rust 测试用 nextest 在 ubuntu-24.04 分三片
  并行，Docker 集成测试只在改到相关路径时跑，iOS 在 xcode-27 镜像（Xcode 27.1，对齐开发机），Android 在
  ubuntu-24.04。草稿 PR 只跑轻检查，转 ready 后才跑重作业；main 不重跑全套，每天定时全量兜底。测试要拉的
  上游镜像每周复制到 ghcr.io/wontonapp/test-images（digest 原样保留），测试经 `WONTON_TEST_IMAGES` 指过去：
  托管 runner 的出口 IP 拉不动 Docker Hub 和 ECR Public。 runner 的要求见
  [.github/workflows/README.md](../../.github/workflows/README.md)。公开仓库的作业对外部的 PR 也会执行：
  权限只给 `contents: read`（需要的作业单独放开），第三方 action 固定到提交 SHA，检出不留凭据。

## 理由

容器镜像和 digest 已经提供了不可变制品和回滚，不需要自建 OTA 体系。

## 代价与风险

- 需要选择并维护国内镜像仓库和两地节点。
- 推送中继需要防滥用：家里服务器注册身份、请求签名、限速。
