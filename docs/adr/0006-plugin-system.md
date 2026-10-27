# ADR-0006 插件系统：薄核心 + Compose 插件

- 状态：已接受（2026-10-28），部分 M0 选型

## 背景

旧的“应用中心”只能观测和启停预装的服务，安装、升级仍靠 installer 和 updater，没有 Docker 应用管理。
目标是“万物皆插件”，管理体验接近飞牛，但以 AI 为主：Agent 能调用插件，也能帮你装插件。

现成的 Docker 应用商店：CasaOS 官方商店（Apache-2.0，约 180 个应用）和 BigBear 商店（MIT，约 490 个）
用 compose 加 `x-casaos` 扩展字段；Umbrel 商店没有开源许可证；Runtipi 商店是 GPL-3.0；
Olares 基于 Kubernetes / Helm，对 Pi 和普通 Linux 太重。

## 决策

- **薄核心**（一个 Rust 容器 `wonton-core`）只负责横切事务：插件管理器（经 Docker API 安装、升级、
  启停、健康检查、日志）、网关与配对（iroh 入口、HTTP 反向代理、设备与用户）、推送代理、MCP 总线、
  密钥库与存储授权。
- **系统插件**（预装、可升级、可替换）：Tuwunel、默认 Agent（Hermes）、模型网关（magpie）。
- **可选插件**：其余一切，如 Immich、Navidrome、Paperless-ngx、Ollama、Syncthing、Samba、下载工具、
  ComfyUI、任意 CasaOS 应用。
- **格式**：一个 `docker-compose.yml` 加顶层扩展字段 `x-wonton`，声明 Web UI 入口、MCP 端点、bot、
  权限、设置表单和资源需求。规范见 [plugin-spec.md](../plugin-spec.md)。
- **生态**：可导入 CasaOS 格式的商店；Umbrel 商店不复用；Runtipi 只允许用户自行添加为商店源，
  不并入官方目录。
- **Agent 调用插件**：统一走 MCP。插件声明 MCP 端点，核心 MCP 总线按权限汇总后提供给 Agent。
- **小程序**：有网页的插件在 App 里以小程序打开，由核心网关鉴权；JS SDK 在 M2 提供。

## 理由

- Compose 是事实标准，任何 compose 加几行元数据就是插件，现成生态可以直接用。
- MCP 是 Agent 工具的事实标准，Hermes、Codex、Claude Code 都原生支持，很多应用已经自带 MCP server。
- 核心只做横切事务，Agent 和模型网关也能整体替换。

## 代价与风险

- Compose 没有多节点调度（家庭场景不需要）。
- CasaOS 应用的权限信息不完整，导入时要补齐并由主人确认。
- 第三方镜像有供应链风险，官方目录需要审核流程（规范的待定问题）。

## 待验证（M0 选型）

- MCP 总线：Docker MCP Gateway（MIT）、IBM ContextForge（Apache-2.0），或基于官方 Rust SDK 自建最小
  聚合。要求：按 Bot 的白名单放行、敏感工具转确认、审计日志。
- HTTP 网关：核心内置（axum / hyper）还是外置 Traefik（MIT，支持按 Docker 标签路由）。
