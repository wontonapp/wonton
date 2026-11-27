# 设计系统的输入

ADR-0026 的令牌、图标白名单和上游 SVG。改完运行 `python3 scripts/generate-design.py`，把生成结果一起提交；
CI 用 `--check` 比对。脚本只要 Python，不下载任何工具。

加图标：在 `icons.toml` 加一项，写上 `kubai` 和 `color`，然后从 npm 取 `icons.toml` 里那个版本的 tarball
（`npm pack kubai-icons-static@<版本>`），运行 `python3 scripts/generate-design.py --vendor-kubai <tarball>`：
它核对 tarball 的校验值，重写 `kubai/` 里的原件和 `SHA256SUMS`。`kubai/` 里的文件不手改，颜色在生成时换。
上游图里有 Compose 的 `ImageVector` 画不了、脚本也没法改写的东西时生成会失败，报出是哪一样。

App 图标和启动屏的来源和生成方式见 [app-icon/](app-icon/README.md)。
