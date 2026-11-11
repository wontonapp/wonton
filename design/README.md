# 设计系统的输入

ADR-0026 的令牌、图标白名单和上游 SVG。改完运行 `python3 scripts/generate-design.py`，把生成结果一起提交；
CI 用 `--check` 比对。生成图标要 Java 21，脚本自己下载固定版本的 Valkyrie 并校验哈希。

加图标：在 `icons.toml` 加一项，把上游的 SVG 放进 `phosphor/`。

App 图标和启动屏的来源和生成方式见 [app-icon/](app-icon/README.md)。
