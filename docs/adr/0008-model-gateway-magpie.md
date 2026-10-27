# ADR-0008 模型网关：magpie

- 状态：已接受（2026-10-28）

## 背景

用户要能自己设置 provider 和 key，而且只配一次就能让 Hermes 和各插件共用，不把 key 散落各处。
magpie（yetone/magpie，MIT，Go）是一个本地模型网关：同时支持 OpenAI Chat / Responses、
Anthropic Messages、Gemini 四种接口并能互相转换；内置 DeepSeek、Kimi、GLM、Qwen、豆包、OpenRouter、
OpenAI、Anthropic、Ollama 等预设；支持订阅账号、额度用尽自动切换和隐私脱敏；有 Docker 镜像和
Web UI；支持列表里有 Hermes Agent。

## 决策

- magpie 作为系统插件预装，锁定版本，可替换。
- 用户在 App 的“模型设置”小程序（magpie 自带的 Web UI）里配置 provider 和 key。
- 核心不自写模型网关，只做两件事：给声明了 `models` 权限的插件注入网关地址和只属于该插件的凭据；
  在网络层只允许这些插件访问 magpie。
- 本地模型插件（如 Ollama）装好后在 magpie 里注册为 provider。

## 理由

不造轮子；国内外主流 provider 和订阅账号现成支持，配置一处、处处可用。

## 代价与风险

- magpie 很新（2026 年 9 月创建）、迭代快、issue 多，必须锁版本，升级前回归。
- 订阅账号登录在容器场景下是否可用，待验证。
- 实时语音接口（WebSocket / WebRTC）不经过它，M2 的实时通话另行处理。

## 待验证（M0）

- 多个插件同时访问时，magpie 的鉴权方式（每插件独立凭据）。
- Hermes 指向 magpie 的配置方式。
