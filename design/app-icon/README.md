# App 图标与启动屏

两处都用主人的原图：`mascot/cutout.png` 是从原图抠出的角色，形状、角度、神态都不改，只缩放、居中。
App 图标把角色放在蓝到白的渐变底上；启动屏只放角色本身，底色是米白和深色。

改完运行 `python3 scripts/make-app-icon.py`（要 Pillow），iOS 的浅色、深色、着色图标、启动屏，
Android 的自适应图标、启动屏和通知栏剪影一起写出，生成结果一起提交。渲染不依赖 Chrome 版本，CI 不比对。
iOS 模拟器会缓存启动屏，看新启动屏要先卸载 App。
