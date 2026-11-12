# 示例插件

用来验证插件规范和核心插件管理器的最小插件，不进入官方目录。

| 目录 | 说明 |
| --- | --- |
| [`grafana/`](grafana/README.md) | Grafana，`sso: oidc`：核心安装时注入 OIDC 客户端配置，打开网页即自动登录。 |
| [`whoami/`](whoami/README.md) | traefik/whoami，回显请求，用来检查网关的鉴权、路由和请求头处理。 |
| [`echo-bot/`](echo-bot/README.md) | 回声 Bot，`matrix.bot`：核心代它登录、建好和主人的加密私聊，它把主人的话原样回过去。 |
| [`gpu-check/`](gpu-check/README.md) | 显卡检查，`devices.gpu`：以非 root 用户在容器里跑 `rocminfo`、`nvidia-smi`，确认核心交给它的显卡能用。 |

安装与端到端验证见 [`deploy/dev/core/README.md`](../../deploy/dev/core/README.md)。
