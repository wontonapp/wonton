# 开发索引

开发和冒烟用的核心镜像（Dockerfile 的 `dev` 阶段）带着这份本地的官方索引（`WONTON_STORE_INDEX`，格式见插件规范的
“官方索引”），代替 GitHub 上的 `geraldpeng6/wonton-store`：商店的官方插件和 Bot 经 MCP 总线能装的插件都来自这里。
`index.json` 和 `blobs/` 由 `update.sh` 生成，不手改；要收的插件和版本列在 `update.sh` 里。

插件仓库是私有的，镜像也没有发布：装之前先在 Docker 主机上按索引记的提交构建镜像，如文件搜索：

```bash
deploy/dev/core/store/checkout.sh geraldpeng6/wonton-plugin-files /tmp/wonton-plugin-files
```

```bash
docker build -t wonton-files:dev /tmp/wonton-plugin-files/images/mcp
```

示例插件 whoami 不在索引里，开发环境和冒烟测试按插件包目录装它（`dev.sh install plugins/examples/whoami`）。
