# 插件

官方插件清单与自研插件源码。每个插件一个目录，即插件包：至少包含带 `x-wonton` 扩展字段的
`docker-compose.yml`，可选图标、设置表单（JSON Schema）和 compose 的 `configs` 引用的文件。
规范见 `docs/plugin-spec.md`。

| 目录 | 说明 |
| --- | --- |
| `hermes/` | 默认 Agent，随核心镜像附带、核心装一次；`config.yaml` 由一次性服务在首次启动前写进数据目录，`wonton/` 是没有模型时提示主人的 Hermes 插件。 |
| `magpie/` | 模型网关，系统插件，由核心预装；常驻服务 default-model 维护默认路由组（ADR-0008），`credentials.sh` 是核心签发、吊销网关凭据时在容器里执行的脚本。 |
| `files/` | 文件搜索：在授权的共享文件夹里找文件、读文件，经 MCP 交给 Agent；`Dockerfile` 构建它的镜像。 |
| `examples/` | 验证规范和插件管理器的最小插件，不进官方索引。 |
