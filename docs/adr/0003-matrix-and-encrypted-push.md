# ADR-0003 消息系统：自托管 Matrix + 加密推送中继

- 状态：已接受（2026-10-28），M0 验证

## 背景

旧方案自研了消息同步（Edge 持久存储加 iroh 同步），三端各写一套同步逻辑，而且没有远程推送：
App 被杀后收不到任何消息。

调研结论：

- 凡是有自有 App 的自托管产品，推送都由厂商运营一个中继，因为 APNs、FCM 的证书只在 App 发行者手里。
  Home Assistant、Nextcloud、Zulip、群晖、ntfy、Rocket.Chat、Matrix/Element 都是这样。
  Nextcloud、Zulip、群晖会先加密通知内容再交给中继；ntfy、Element 只发消息 ID。
- 商业 IM 云（如腾讯云 IM）推送和厂商通道现成，但消息正文在第三方云上，按用量计费
  （专业版国内 1499 元/月、境外 2999 元/月起），境内外要分别建应用，和“数据在家”矛盾。
- 国内 Android 必须接厂商通道；小米、vivo、OPPO（正式权限）要求企业开发者账号和商店上架。

## 决策

- 每台服务器内置 Tuwunel（Rust 写的 Matrix homeserver），默认关闭联邦。它是会话、历史、多端同步、
  家人群聊的唯一事实源。
- 客户端用 matrix-rust-sdk，同步用 Simplified Sliding Sync（Tuwunel 已支持 MSC4186）。
- Agent 和插件都以 Matrix 用户（bot）的身份收发消息。
- 推送链路：Tuwunel 的 HTTP pusher 指向核心推送代理；核心用目标设备的公钥加密通知内容，再交给我们
  运营的推送中继（gorush，支持 APNs、FCM、华为 HMS）；中继只看到密文。iOS 用通知扩展解密显示，
  Android 在推送服务里本地解密。
- 国内 Android：主体和上架完成前，提供可选的“保持连接”模式（前台服务）；之后接厂商通道（M2）。

## 理由

- 用标准协议和现成实现替代自研同步：历史、多端、已读、附件、群聊、通话标准都是现成的。
- 正文只存在家里；云端只多了一个无状态的中继。
- Matrix 生态可复用：Hermes 已有 Matrix 平台，mautrix 系列能桥接其他 IM，Element 可作为备用客户端。

## 代价与风险

- 要学 Matrix 的房间和事件模型；Agent 的流式输出要用消息编辑或旁路实现。
- Tuwunel 是社区项目，不是 Element 官方出品；大量加入联邦大房间时出现过性能问题（我们默认不开联邦）。
- 推送中继的滥用防护要自己做：服务器注册、令牌、限速。
- Element X、FluffyChat 和 Dart 版 Matrix SDK 是 AGPL，不能使用；只用 Apache-2.0 的 matrix-rust-sdk。

## 待验证（M0）

- Tuwunel 经隧道同步的性能，JWT 登录签发，pusher 行为。
- iOS 通知扩展的内存限制下解密的可行性（优先用系统自带的加密能力，不在扩展里跑完整核心）。
- Hermes Matrix 平台在 Tuwunel 上的表现。
