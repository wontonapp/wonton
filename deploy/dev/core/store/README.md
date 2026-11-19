# 开发索引

开发和冒烟用的核心镜像（Dockerfile 的 `dev` 阶段）带着这份本地的官方索引（`WONTON_STORE_INDEX`，格式见插件规范的
“官方索引”）。起栈时把 shell 的 `WONTON_STORE_INDEX` 设成 `/usr/share/wonton/store` 才用它；不设时核心读 GitHub 上的
`wontonapp/wonton-store`。商店的官方插件和 Bot 经 MCP 总线能装的插件都来自生效的那份索引。
`index.json` 和 `blobs/` 由 `update.sh` 生成，不手改；要收的插件和版本列在 `update.sh` 里，`onboarding` 段跟着
wonton-store 的固定提交（去掉 `agents`），脚本校验引导引用的插件都在索引里之后才写出索引。

示例插件 whoami 不在索引里，开发环境和冒烟测试按插件包目录装它（`dev.sh install plugins/examples/whoami`）。
