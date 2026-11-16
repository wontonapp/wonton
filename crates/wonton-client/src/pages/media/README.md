# 插件页媒体

Android WebView 不导出网页 Media Session，壳用 Media3 转接系统控制，不替代网页播放器。
iOS 注入本模块脚本，只在播放状态变化时调 `page_media_active`：放歌退到后台，核心照常探测、
自动重连；锁屏控制条由 WebKit 自己给（Media Session API），壳不再接一份。

1. 主框架文档开始前注册 `wontonMedia` 字符串通道，限制到插件 origin，注入
   `ClientCore.page_media_script()`。非主框架消息不处理。
2. 通道消息交给 `page_media_message()`；`None` 不改状态。版本 1 的 JSON 由脚本产生，
   时间为毫秒；封面是同源、带 Cookie 获取并缩到最长边 512 的 JPEG base64，最大 512 KiB。
3. 最后一个报告 Playing 的 WebView 持有进程内媒体会话。系统操作通过
   `page_media_command_script(action, seek_ms)` 原样注入该 WebView；参数为毫秒，
   快进快退没给参数时用 10 秒。没有 handler 时仅 play、pause、seekto 操作当前媒体元素。
4. Playing 才启用媒体前台服务与唤醒锁；暂停保留可划掉的通知。销毁页面时摘掉通道和文档脚本，
   释放属于该 WebView 的会话与通知。来电焦点由 WebView 处理。

Media3 1.8.0（Apache-2.0）于 [2025-07-30 发布](https://github.com/androidx/media/blob/1.8.0/RELEASENOTES.md#180-2025-07-30)。
行为验证：`cargo test -p wonton-client --locked --test it tabs_media`；
脚本往返用固定摘要的官方 Node 22 镜像，显式传 `--ignored` 运行。
