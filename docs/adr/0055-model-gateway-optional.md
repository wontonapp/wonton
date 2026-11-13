# ADR-0055 模型只有一个入口：官方订阅直连，其余经 magpie

- 状态：已接受（2026-11-07）；2026-11-13 按主人决定修订模型来源边界，配套入口见 [ADR-0034](0034-onboarding-and-help.md)，网关契约见 [ADR-0008](0008-model-gateway-magpie.md)

## 修订记录

- 2026-11-13：保留官方订阅登录；将“有网关就用，否则自行配 provider”收敛为下述两种模型来源，移除缺网关时的隐藏直连。原生模型配置入口的修订见 ADR-0034。

## 背景

官方订阅的额度可能只能在对应 Agent 的官方程序里使用，不能要求全部经过模型网关。但 API key、自建模型和外部网关分散在各 Agent 里配置，会让主人找不到统一的切换、测试和用量入口。

主人原话（2026-11-13）：

> 模型只有一个入口
>
> 用户得有个地方方便切模型测试模型存活呀，这些不是 magpie 都自带的吗？

主人选择“官方登录保留，其余全走网关”；open-webui、n8n、SillyTavern 这类非 Agent 应用“先不纳入”强制收口。

## 决策

### Agent 的模型来源只有两种

- **官方订阅登录。** 主人明确选择后，在 Agent 官方程序内完成登录，模型请求直连官方；登录入口和条款边界见 [ADR-0034 的“接模型”](0034-onboarding-and-help.md#接模型)。这不是网关故障时的后备路径，也不包括直接填写官方 API key。
- **magpie 网关。** 其余所有模型，包括 API key、自建服务、主人 Mac 上的网关，都只在 magpie 配置；Agent 使用核心注入的稳定网关地址和受限凭据。上游登记与默认路由见 ADR-0008。

Agent 插件内能改 provider 的入口（如 Hermes 的 `/model`），默认值和提示都指向 magpie；不再把直接填写外部 key 或地址作为第三种受支持的接法。Wonton 的引导不另做服务商、key、自建地址表单，统一打开 ADR-0034 规定的模型设置网页。

### 缺网关就如实报未就绪，不偷偷换路

`models` 是访问模型网关的权限，不是“有就用、没有就直连”的选路开关。选择网关的 Agent 在网关未安装、未就绪、不可达或凭据签发、验证失败时，必须如实报未就绪或对应原因，不发起替代的直连模型请求。

不因空的 `WONTON_MODEL_*`、已保存的用户 key 或已有官方登录态，自动改连官方、自建服务或外部网关。反过来，主人选择官方订阅登录后，也不自动切到网关。magpie 内部按路由组顺序切换上游是公开配置的路由，不是这种隐藏回退；规则见 ADR-0008。

仍可先安装插件、跳过接模型；安装成功和容器健康不等于模型可用。官方订阅登录不要求 magpie 就绪。网关凭据的签发、注入、对账和吊销沿用 ADR-0008 的通用契约，不为具体 Agent 增加核心特例。

### 非 Agent 应用只统一默认值

open-webui、n8n、SillyTavern 等非 Agent 应用安装后默认指向 magpie，主人可以在应用里自行更改。不禁止它们自配 provider，也不把这些应用的强制收口算进本次决定。

## 理由

- 只有 magpie 管理非订阅模型，上游或地址变化只改一处，切换、测试、用量复用它的原生能力。
- 官方订阅保留官方用法，不把订阅凭据或请求中转进 magpie。
- 缺能力时明确失败，避免主人以为在测网关，实际请求却去了别处；保留先安装、后接模型的流程。

## 没选的做法

- 所有订阅也转给 magpie：破坏官方登录边界，条款风险见 ADR-0034。
- 缺网关时改用官方或用户 key，或注入假地址、占位凭据：隐藏选路或伪装成功，违背“失败即停”。
- 为切模型、测试和看用量另造 App 页面：重复 magpie 的能力，并产生第二套模型配置入口。

## 代价

- Agent 的主模型、辅助模型和可改 provider 的入口都受同一来源规则约束，不能只改首次启动的默认值。
- 只注入网关变量不能证明所有请求都经过网关；允许出网、能执行代码的插件仍有独立网络能力。模型来源约定不等于已验证的网络级强制隔离，不能据此作后者的保证。
- 模型存活测试可能发出真实请求并计费；magpie 的上游回退也不能在已向客户端发送回复内容后保证换路。能力与限制见下方固定版本来源。

## 影响范围与边界

2026-11-13 的源码调研以核心提交 `ef0453434ab29e7bfeaa0a3cbcc960d1faba1ee7` 和下列插件版本为依据。这里只定位职责模块，不规定实施顺序，也不把调研当作端到端实测。

| 范围 | 涉及的插件或模块 |
| --- | --- |
| Agent 插件 | [Hermes 配置与初始化](https://github.com/wontonapp/wonton-plugin-hermes/blob/da601989c34600bc36262275ccba42f3e76468c8/config.yaml)、[OpenClaw 配置](https://github.com/wontonapp/wonton-plugin-openclaw/blob/e3801634e9c5d36e51da926ffaf2ac02be65351b/docker-compose.yml)；主、辅助模型及 provider 切换入口 |
| ACP Agent 插件 | `wonton-agents` 的 omp、Codex、Claude Code、OpenCode、Qwen Code、Kimi Code、DSH 及[共用网关适配](https://github.com/wontonapp/wonton-agents/blob/base/v0.1.1/base/wonton_agent/gateway.py)；其中 [Codex 认证选路](https://github.com/wontonapp/wonton-agents/blob/523a241f4a1b3f2b0b85f6c34d61bdbeed3dd27d/agents/codex/image/gateway)、[DSH 缺网关分支](https://github.com/wontonapp/wonton-agents/blob/4492982e5910d1458641cbdafb76b503821f94ef/agents/dsh/image/gateway)属于来源边界；不请求模型的 Echo 不受影响 |
| 其他 Agent 适配 | [agent-orchestrator 的工人模型配置](https://github.com/geraldpeng6/wonton-plugin-agent-orchestrator/blob/cf6c1afed994d88c1828908371522c72b34c96ed/images/wonton-plugin-agent-orchestrator/gateway)、[ZCode 网关与认证选路](https://github.com/geraldpeng6/wonton-plugin-zcode/blob/24307445ca37f5b875fc84cf9785e5bfea04f9de/images/wonton-plugin-zcode/gateway) |
| 核心与协议 | `wonton-core` 的插件模型凭据、变量注入、清单及能力校验、模型转发；`wonton-protocol` 的引导模型配置契约 |
| 客户端与两端壳 | `wonton-client` 的 `onboarding`、侧栏及插件网页导航；iOS 的 `Onboarding*`、`MainView+Sidebar`，Compose 的 `Onboarding*`、`AppNavigation`、`LauncherScreen` |
| 模型设置与推荐索引 | magpie 的 Providers、Routing、Usage 和默认组；`wonton-store` 的引导服务商配置项。网页能力依据为 [magpie 插件 v0.6.2](https://github.com/geraldpeng6/wonton-plugin-magpie/blob/a8566ba2596f7ca15f9b4ed85ffd186523dd5c27/docker-compose.yml)固定的 0.1.1327 及[上游同版本源码说明](https://github.com/yetone/magpie/blob/398803e3f575dcafc20573f674a98d71105c9694/docs/reference.md)；源码与发布镜像构建来源的完全对应未核实 |

本次决定不包含状态提前看得见（B）、出错卡片（C）、核心自愈（D），也不包含非 Agent 应用的强制收口。存活测试的真实请求见[上游 Providers 测试](https://github.com/yetone/magpie/blob/398803e3f575dcafc20573f674a98d71105c9694/internal/gui/providers.go#L1836-L1859)，回退限制见[上游路由说明](https://github.com/yetone/magpie/blob/398803e3f575dcafc20573f674a98d71105c9694/docs/subsystems/gateway-routing.md)。

## 验证结果（2026-11-07，真实栈）

以下记录对应 2026-11-13 修订前的安装、空变量注入和对账机制，不证明上述模型来源约束或网页引导已通过端到端验证。

环境：magpie 0.1.1327、Hermes Agent v0.22.5、Tuwunel v1.9.4，`deploy/dev/core/dev.sh`（项目 `wonton-pmo`）。
复跑：起栈（`WONTON_DEV_DEFAULT_PLUGINS=` 空，不自动装 Hermes）→ `dev.sh uninstall ai.usemagpie.magpie --purge` →
`dev.sh install plugins/hermes --yes` → 重启核心（系统插件重装 = 装上网关）→ 拆栈。

- 没网关装 Hermes：装得上，状态 Running；详情页 `hints` 是 `[{"category": "models_not_connected", "detail": null}]`；
  运行容器里 `WONTON_MODEL_BASE_URL`、`WONTON_MODEL_API_KEY`、`WONTON_MODEL_DEFAULT` 都定义且为空字符串。
- 重启核心装上网关：日志「模型网关接上了，使用方已按新变量重建」，Hermes 容器按新变量重建，
  三个变量有值，签发的凭据请求 `GET <base>/models` 返回 200，详情页 `hints` 变空。
- 再卸掉网关：使用方不动、仍 Running，凭据值原样留在容器里，详情页提示变回「没接上」。
- 网关就绪时已接上的使用方不重建：假网关行为测试
  （`the_gateway_coming_back_again_leaves_connected_consumers_alone`）。真实栈里发现过对账把已接上的
  使用方也重建了一遍，已修：只有没接上的（没凭据或记着签发问题）才签发并重建。

## 待验证

- 官方订阅登录不依赖网关；网关模式下，缺网关、凭据失败、已有官方登录态或用户 key 都不会触发隐藏直连。
- Agent 的首次配置、持久配置、主/辅助模型和切换入口遵循同一规则；非 Agent 应用仍可自行更改模型来源。
- 从引导和侧栏进入同一个 magpie 网页，配置上游、切换路由、测试存活和查看用量；验收入口见 ADR-0034。
