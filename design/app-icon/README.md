# App 图标与启动屏

App 图标用主人的原图：`mascot/cutout.png` 是从原图抠出的角色，形状、角度、神态都不改，只缩放、居中放在蓝到白的渐变底上。
改完运行 `python3 scripts/make-app-icon.py`（要 Pillow），iOS 的浅色、深色、着色和 Android 自适应图标的前景、背景、单色一起写出，生成结果一起提交。

启动屏是甲骨文“云”，墨色字配米白底。字形来自 Wikimedia Commons 的
[File:云-oracle.svg](https://commons.wikimedia.org/wiki/File:%E4%BA%91-oracle.svg)，作者在文件页声明放弃版权、进入公有领域。

- `yun-oracle.svg`：上游原文件，不改。
- `icon.svg`：启动屏母版，颜色和摆放都从这里读。字形的路径必须和上游一致，生成脚本会比对。

改完启动屏运行 `python3 scripts/generate-app-icon.py`（要 macOS 和 Chrome），把生成结果一起提交。渲染依赖 Chrome 版本，
CI 不比对。iOS 模拟器会缓存启动屏，看新启动屏要先卸载 App。
