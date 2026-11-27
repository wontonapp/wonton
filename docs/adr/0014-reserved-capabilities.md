# ADR-0014 预留能力：实时通话、音乐、渠道、Web

- 状态：已接受（2026-10-28）。“小程序 JS SDK”一节已被 [ADR-0033](0033-plugin-tabs.md) 取代；“Subsonic 原生音乐页”一节先撤销（主人 2026-11-06 定）、后由 [ADR-0113](0113-media-transport.md) 重新定回原生（主人 2026-11-21 定）；Web 端的时机和渲染由 [ADR-0124](0124-client-react-pwa-electron.md) 定

MVP 只做主干。以下能力现在就在架构上留好位置，之后按耦合程度拆成独立任务并行开发。

## 实时通话与屏幕共享（M2）

目标：和 Bot 语音通话，App 在后台也不断线；通话中可以把屏幕共享给 Agent（类似豆包的打电话加共享屏幕）。

- **底座**：LiveKit（Apache-2.0）。Matrix 2.0 的通话标准 MatrixRTC 也基于 LiveKit，所以“给 Bot
  打电话”就是一通 Matrix 通话，Agent 作为参与方加入。Tuwunel 已部分支持 MatrixRTC。
- **默认：云端实时模型直连**。家里服务器签发临时令牌，手机直接连实时接口（豆包、通义 Qwen-Omni、
  智谱 GLM-Realtime、OpenAI、Gemini Live 等），延迟最低；需要工具时回家里调 Agent。
- **隐私模式**：家里跑 LiveKit 插件，用本地模型做语音识别和合成（适合带 GPU 的主机）。媒体流需要
  经 iroh 传输 UDP，要先做技术验证。
- **屏幕共享**：iOS 录屏扩展（Broadcast Upload Extension），Android MediaProjection；画面作为通话里
  的一路视频交给多模态模型；必须有明确的授权提示。
- **后台**：iOS 开后台音频模式；海外可以用 CallKit 和 VoIP 推送，**中国大陆不能用 CallKit**，改用实时
  活动或通知显示“通话中”，Agent 主动来电也退化为普通通知。Android 用前台服务（麦克风、录屏类型）
  配合系统通话管理接口。
- **预留**：客户端核心把音频会话和后台模式抽象一次，供通话和插件网页里的音频共用。

## Subsonic 原生音乐页（M2）

2026-11-06 撤销过一次（不做原生音乐页，音乐用插件网页放），2026-11-21 主人又定回来了：播放、队列、正在播放、专辑页两端原生做，Navidrome 只当曲库与 Subsonic/OpenSubsonic 接口。底座的结论见 [ADR-0113](0113-media-transport.md)（会话与队列在服务器核，壳出声），播放界面的方案见影音相册线的音乐块。

## 小程序 JS SDK（M2）

已被 [ADR-0033](0033-plugin-tabs.md) 取代：不做 `window.wonton`，用 MCP Apps 的方法名，原生能力以后按那里的表扩展。

## 第三方 IM 渠道（M2）

Telegram、飞书、企业微信等作为可选的渠道插件，基于 Hermes 自带的平台或 mautrix 桥。

## 桌面端（第二期）与 Web 端（第一期 PWA）

桌面用 Electron + React，与 PWA 共用 React 渲染层；客户端核心是本机旁路进程，本机服务供手机远程控制（[ADR-0124](0124-client-react-pwa-electron.md)）。
Web 第一期做 PWA，同一套 React 渲染层，每用户一份核心、所有窗口共用，网页由家里提供（详见 [ADR-0124](0124-client-react-pwa-electron.md)）。

## 跨机 Bot 通信（M3）

独立插件，Matrix 联邦走 iroh，或 A2A 协议；两台服务器先配对，再按 ADR-0007 的权限规则互通。
