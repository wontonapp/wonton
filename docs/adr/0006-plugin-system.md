# ADR-0006 插件系统：薄核心 + Compose 插件

- 状态：已接受（2026-10-28），M0 选型已定（见验证结果）。系统插件的定义以 [ADR-0007](0007-agents-and-bots.md) 为准；“导入 CasaOS”已撤销（主人 2026-11-04 定）；“JS SDK”已被 [ADR-0033](0033-plugin-tabs.md) 取代；“以后用 rustls 补上”已被取代，见下文

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
- **系统插件**：`kind: system`，随核心预装，主人换不掉，如模型网关 magpie（ADR-0007）。Hermes 是默认装一次的普通插件，Tuwunel、Hydra 是核心组件。
- **可选插件**：其余一切，如 Immich、Navidrome、Paperless-ngx、Ollama、Syncthing、Samba、下载工具、
  ComfyUI。
- **格式**：一个 `docker-compose.yml` 加顶层扩展字段 `x-wonton`，声明 Web UI 入口、MCP 端点、bot、
  权限、设置表单和资源需求。规范见 [plugin-spec.md](../plugin-spec.md)。
- **生态**：不导入 CasaOS，热门应用转成官方插件；Umbrel 商店不复用；Runtipi 只允许用户自行添加为商店源，
  不并入官方目录。
- **Agent 调用插件**：统一走 MCP。插件声明 MCP 端点，核心 MCP 总线按权限汇总后提供给 Agent。
- **小程序**：有网页的插件在 App 里以小程序打开，由核心网关鉴权；原生能力见 ADR-0033。

## 理由

- Compose 是事实标准，任何 compose 加几行元数据就是插件，现成生态可以直接用。
- MCP 是 Agent 工具的事实标准，Hermes、Codex、Claude Code 都原生支持，很多应用已经自带 MCP server。
- 核心只做横切事务，Agent 和模型网关也能整体替换。

## 代价与风险

- Compose 没有多节点调度（家庭场景不需要）。
- 第三方镜像有供应链风险，官方目录需要审核流程（规范的待定问题）。

## 待验证（M0 选型）

- MCP 总线：Docker MCP Gateway（MIT）、IBM ContextForge（Apache-2.0），或基于官方 Rust SDK 自建最小
  聚合。要求：按 Bot 的白名单放行、敏感工具转确认、审计日志。
- HTTP 网关：核心内置（axum / hyper）还是外置 Traefik（MIT，支持按 Docker 标签路由）。

## 验证结果（M0-6）

三项结论：插件管理器直接用 bollard 渲染 compose 子集；HTTP 网关内置在核心里（axum + hyper）；
MCP 总线用官方 Rust SDK rmcp 在核心里做最小聚合（M1 实现）。验证环境：本机 macOS + OrbStack（arm64，
Docker 29.8）和一台 Linux 开发机（x86_64，Docker 29.7，`DOCKER_HOST` 指向它）。端到端可复跑：
`deploy/dev/core/smoke.sh`，两台都通过（安装 Grafana 示例插件约 6 秒，含健康检查；卸载 1–10 秒）。

**插件管理器**（`crates/wonton-core/src/plugin/`）

- 经 Docker API 直接创建网络、卷和容器，不依赖 docker compose 命令行。核心只接受 Compose 子集
  （白名单，见插件规范）：未知字段报错，端口照旧拒绝；宿主网络、特权、设备、Docker socket 等高权限字段
  要声明对应权限（ADR-0036）。YAML 用 serde-saphyr 解析（纯 Rust，带资源预算，支持锚点和合并键）。
- 安装：校验清单 → 核对主人确认的权限 → 用占位环境预渲染 → 拉镜像 → 建应用数据目录并交给
  PUID:PGID → 建插件网络并把核心接进去 → 建命名卷 → 登记 OIDC 客户端 → 按 `depends_on` 启动 →
  等健康检查或稳定运行 → 探测网页入口。任一步失败按相反顺序回滚，错误里附容器最近的日志。
- 卸载按标签 `wonton.plugin=<id>` 清理容器、网络和 OIDC 客户端，选择删除数据时再删卷和应用数据目录；
  删完再按标签查一遍，有残留就报错。核心重启后按记录恢复：核对容器在运行、重新接入插件网络、
  把 OIDC 客户端同步到身份提供方；不一致的插件标为异常并给出原因（冒烟脚本验证）。
- 代价：compose 语义要自己维护一个子集，遇到新字段要逐个评估后再加。这是有意的：
  白名单让核心完全掌握插件能拿到的宿主能力，未知字段不会被悄悄执行。

**HTTP 网关：核心内置**

实验 `deploy/dev/core/experiments/gateway-bench.sh`：同一个插件（whoami），在插件网络内用 oha
压测 10 秒、16 个连接（OrbStack arm64）：

| 路径 | 请求/秒 | p50 | p99 | 网关内存 |
| --- | --- | --- | --- | --- |
| 直连插件（基线） | 约 28000 | 0.10 ms | 7.6 ms | — |
| 核心内置网关（校验会话、剥离网关 Cookie、重写转发头） | 约 21000 | 0.27 ms | 5.2 ms | 12 MiB（整个核心） |
| Traefik v3.6 + forwardAuth（每个请求先问核心） | 约 5800 | 1.16 ms | 21.8 ms | 36–104 MiB |

Traefik 一组在负载下有约 0.7% 的 500（forwardAuth 子请求失败，配置未调优）。选内置的理由：

- 设备身份来自 iroh 隧道（ADR-0004）。内置网关可以在进程内拿到隧道已认证的身份；用 Traefik 只能每个
  请求 forwardAuth 回调核心，多一跳，身份还要写进请求头再信任。
- 只有核心能挂 Docker socket（ADR-0011），Traefik 的 Docker provider 用不了，只能由核心生成路由配置，
  路由就有了两份事实源。
- 少一个容器，小主机上更省内存。
- 代价：反向代理自己维护（`gateway/proxy.rs`），行为测试覆盖鉴权、Cookie 剥离、转发头和协议升级；
  Grafana 的 WebSocket 经网关可用。转发在请求体发完后等响应头最多 120 秒，超时回 504 `upstream_timeout`，连接不上回 502 `upstream_unreachable`；网页入口的错误按 `Accept` 协商，页面导航回 HTML 错误页，接口（`fetch`、`*/*`）回 JSON 并带 `x-wonton-error`，Matrix、模型网关和 tusd 各按自己的 JSON 格式回（tusd 超时沿用 `service_unavailable`，客户端已有暂停重试语义）；
  Matrix 客户端接口（长轮询）、协议升级和上传不计入，响应头之后的流式转发仍然不限时。“TLS 和 ACME 以后用 rustls 补上”已被取代：经隧道不另加 HTTPS（ADR-0009），面向第三方 App 的 TLS 在入口容器（ADR-0021）。

**MCP 总线：rmcp 最小聚合**

三个候选接同一个插件 MCP 服务器（streamable HTTP；`search_photos` 普通，`delete_photo` 敏感）。

| | Docker MCP Gateway v0.44.3 | IBM ContextForge v1.1.0 | rmcp 3.8 最小聚合 |
| --- | --- | --- | --- |
| 按 Bot 白名单 | 不支持：全实例一个共享令牌，`--tools` 对所有客户端生效，每个 Bot 要单独起一个实例 | 支持：每个 Bot 一个虚拟服务器，令牌限定到该服务器（实测越权被拒） | 支持：Bot 令牌对应白名单，过滤 `tools/list`、拦截 `tools/call` |
| 敏感工具转确认 | 拦截器是唯一钩子；实测 `exec`、`http` 拦截器对 streamable HTTP 客户端的调用都失败（`marshalling request: json: unsupported type: func(mcp.CloseSSEStreamArgs)`），且拿不到调用方身份 | 没有内置，要写 Python 插件挂 `tool_pre_invoke` | 挂起调用，等主人确认后再转发 |
| 审计日志 | 只有 stderr 日志 | 审计表记录管理操作，工具调用只有指标 | 每次调用一行 JSON（Bot、工具、参数、决定） |
| 部署约束 | 只连远程服务器也必须挂 Docker socket，否则启动即退出；远程地址默认必须是 https | 5 个必填密钥；镜像 585 MB；空载内存 417 MiB（1 个 worker，默认 worker 数下 2.6 GiB） | 在核心进程内，不加容器 |

复跑：`deploy/dev/core/experiments/mcp-bus/`（rmcp 实验约 200 行，`run.sh` 演示白名单、确认和审计；
另外两个候选的步骤写在该目录的 README）。选 rmcp 自建的理由：

- Docker MCP Gateway 违反“只有核心挂 Docker socket”，也没有按调用方区分权限的能力。
- ContextForge 能做白名单，但确认要用另一种语言写插件，且对树莓派这类主机太重。
- 核心本来就掌握插件的 MCP 端点（清单）、插件网络和 Bot 身份；按 Bot 放行、聊天里的确认卡片、
  审计都放在核心里最直接，协议细节交给官方 SDK。
- rmcp 支持 MCP 2026-07-28，包括 Tasks 扩展：M1 里敏感调用可以先返回任务，主人确认后再完成，
  不必一直占着 HTTP 请求。

**插件规范的调整**（已写进 `docs/plugin-spec.md`）：`webui.oidc`（回调路径、令牌端点认证方式）；
新增 `WONTON_APP_URL` 和三个 OIDC 端点变量；变量只用于插值，未定义即安装失败；Compose 子集；
网页地址 `<slug>.<基础域名>`；M0 只兑现 `storage.share.*` 权限。

这一组只在本机浏览器验证；App 内 WebView 经隧道打开小程序的验证见 ADR-0004。
