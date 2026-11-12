# App 图标

甲骨文“云”，墨色字配米白底。字形来自 Wikimedia Commons 的
[File:云-oracle.svg](https://commons.wikimedia.org/wiki/File:%E4%BA%91-oracle.svg)，作者在文件页声明放弃版权、进入公有领域。

- `yun-oracle.svg`：上游原文件，不改。
- `icon.svg`：母版，颜色和摆放都从这里读。字形的路径必须和上游一致，生成脚本会比对。
- `icon-small.svg`：iOS 40pt 及以下用的母版，只加粗描边、略微缩小，路径不变。

改完运行 `python3 scripts/generate-app-icon.py`（要 macOS 和 Chrome），把生成结果一起提交。渲染依赖 Chrome 版本，
CI 不比对。iOS 模拟器会缓存启动屏，看新启动屏要先卸载 App。
