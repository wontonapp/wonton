# 插件

官方插件清单与自研插件源码。每个插件一个目录，即插件包：至少包含带 `x-wonton` 扩展字段的
`docker-compose.yml`，可选图标、设置表单（JSON Schema）和 compose 的 `configs` 引用的文件。
规范见[公开的插件规范](https://github.com/geraldpeng6/wonton-docs/blob/main/plugin-spec.md)。别的官方插件各在自己的仓库里，由官方索引收录，如文件搜索
`geraldpeng6/wonton-plugin-files`。

| 目录 | 说明 |
| --- | --- |
| [`hermes/`](hermes/README.md) | 默认 Agent，随核心镜像附带、核心装一次；`config.yaml` 由一次性服务在首次启动前写进数据目录，`wonton/` 是没有模型时提示主人的 Hermes 插件。 |
| [`magpie/`](magpie/README.md) | 模型网关，系统插件，由核心预装；常驻服务 default-model 维护默认路由组（ADR-0008），`credentials.sh` 是核心签发、吊销网关凭据时在容器里执行的脚本。 |
| [`examples/`](examples/README.md) | 验证规范和插件管理器的最小插件，不进官方索引。 |

## 自制图标

echo-bot、gpu-check 和 whoami 的 `icon.svg` 由本仓库手写绘制，不使用第三方标志，随[仓库许可](../README.md#许可)保留所有权利。
上游官方标志的来源和商标归属见对应插件的 README，许可声明随插件包（`notices`）。
