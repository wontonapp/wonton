# 插件

本目录只保留示例插件和预装插件锁文件。每个官方插件各在自己的仓库，仓库根目录是插件包：至少包含带
`x-wonton` 扩展字段的 `docker-compose.yml`，以及清单引用的文件。规范见
[公开的插件规范](https://github.com/geraldpeng6/wonton-docs/blob/main/plugin-spec.md)。

| 目录 | 说明 |
| --- | --- |
| [`system.lock`](system.lock) | 核心镜像内置的系统插件和默认插件的来源、提交与插件包摘要。 |
| [`examples/`](examples/README.md) | 验证规范和插件管理器的最小插件，不进官方索引。 |

## 预装插件

模型网关 [magpie](https://github.com/geraldpeng6/wonton-plugin-magpie) 是系统插件；默认 Agent
[Hermes](https://github.com/geraldpeng6/wonton-plugin-hermes) 只在首次启动时安装，主人卸载后不再装回。
核心镜像构建时运行 `wonton-core plugin fetch-locked plugins/system.lock <目标目录>`，只取插件包文件，核对提交与
ADR-0017 定义的包摘要；包随镜像发布，首次启动不从 GitHub 取包。

锁文件每行以空白分隔：`system|default 名称 github:owner/repo@引用 40位提交 sha256:摘要`。
升级只改对应插件的一行（来源引用、固定提交和包摘要），然后重建核心镜像；开发期先以 40 位提交号作引用，
仓库所有者审核并打 tag 后再改为显式 tag，保留对应提交与摘要，不使用浮动分支或最新版本。

取包目标目录必须不存在，失败时删除本次输出，不修改其他目录。构建期命令读取 `https_proxy`（其次 `HTTPS_PROXY`）；
运行中的核心仍只使用主人的出网代理设置。

## 模型网关契约

`crates/wonton-core/src/plugin/tests/model_gateway_contract/` 把消费方契约与提供方适配器分开。
消费方只使用注入的地址、凭据和不透明的默认模型 ID；测试替身故意使用不同的命令、接口基础路径和模型 ID。
系统网关适配器读取 `system.lock` 对应提交的包并核对摘要，执行包内声明的凭据、服务商管理命令和默认模型维护服务。
这证明实现之间的接口约束，不改变系统插件的安装或替换规则。

```sh
cargo test -p wonton-core --lib model_gateway_contract
cargo test -p wonton-core --lib model_gateway_contract -- --ignored
```

第一条不依赖 Docker。第二条需要 Docker、GitHub 取包及锁定的官方镜像；运行时的模型上游只在测试进程里，
不使用真实服务商、宿主凭据或已有网关的数据。容器在独立的内部网络里，测试守卫清理本次创建的资源。
契约覆盖模型列表、Chat Completions 与 Responses 的文本、SSE 及工具调用往返、图像消息，
以及服务商测试不落盘、被拒不保存、断连保存但如实报错和凭据隔离、对账、吊销。
它不验证模型回答质量，也不等同于完整 Agent 的端到端冒烟。

## 自制图标

echo-bot、gpu-check 和 whoami 的 `icon.svg` 由本仓库手写绘制，不使用第三方标志，随[仓库许可](../README.md#许可)保留所有权利。
上游官方标志的来源和商标归属见对应插件的 README，许可声明随插件包（`notices`）。
