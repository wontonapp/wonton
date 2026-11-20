# ADR-0114 插件 Agent 底座：家人账号、群聊说话人、事件、按人通知、卡片模板、深链、结果可见范围、文件交给插件

- 状态：提议（2026-11-21），影音相册线起草，插件底座线复审后合并
- 关联：[ADR-0015](0015-small-components-standard-interfaces.md)（小部件、标准接口）、[ADR-0016](0016-mcp-bus-and-confirmations.md)（MCP 总线与确认）、
  [ADR-0020](0020-file-service.md)（文件服务）、[ADR-0024](0024-confirmation-scope.md)（每次问的三类）、[ADR-0038](0038-multi-step-cards.md)（卡片）、
  [ADR-0041](0041-app-links-and-shortcuts.md)（`wonton://` 链接）、[ADR-0045](0045-family-members.md)（家人）、[ADR-0047](0047-plugin-notices.md)（插件通知）、
  [ADR-0088](0088-family-accounts.md)（角色与按人身份）、[ADR-0097](0097-share-in.md)（分享进来）、[ADR-0104](0104-view-file-media.md)（看文件）、
  [ADR-0108](0108-hermes-default-kernel.md)（Hermes）、[ADR-0110](0110-one-hermes-container-per-person.md)（一人一个 Hermes 容器）
- 边界：[ADR-0113](0113-media-transport.md) 管长任务（备份上传、播放会话、离线下载）；本篇只管 Agent 用它时要的通用接口，不重写它的模型

## 背景

相册、音乐、影视三份方案各自拆出了一批“插件底座要先补的接口”，大半是同一件事（`notes/summary.md` 的“三块共用的底座项”表）。
它们有一个共同的形状：插件替某个家人在另一个应用里办事，而现在的总线只认得住“哪个 Bot”，认不出“替谁”：

- 家人的 Wonton 账号和应用里的账号没有对应关系，插件只能全家用一把 key（相册 F1、音乐 K6、影视卡 3）。
- 共享 Bot 在群里办事时，插件拿不到说话人（相册 F2/卡 3b、音乐 K0b④、影视卡 7）。
- 插件有事要说时，只能发不可回话的通知，发不到指定的人，也叫不醒空闲的 Agent（相册 F4、音乐 K8、影视卡 15/17）。
- 确认卡片只有标题加原始 JSON，结果文件只有 Bot 的归属人能看，通知只能到主人，插件拼不了能点开的深链，插件的文件进不了插件。

Agent 是 Hermes，一个家人一个容器（[ADR-0108](0108-hermes-default-kernel.md)、[ADR-0110](0110-one-hermes-container-per-person.md)）：每人的 Bot 分容器，
同一个人的多个 Bot 同容器、以 profile 分开。轮到谁的 Bot 说话，`X-Wonton-On-Behalf-Of` 就带谁，核心已经拿它算 AI 动手级别
（`plugin-spec.md:2456-2462`、`mcp/audience.rs:105-120`）。本篇把它接到的身份继续往下递。

## 已有的，不重建

下面八项里，有六项已经有了一半，只补缺口，不另起一套：

| 已有 | 在哪 | 本篇只补 |
| --- | --- | --- |
| 按人的网页登录（OIDC `sub` 就是本机用户名，代理登录模板带 `{user}`） | `plugin-spec.md:710-760`、`:861-880` | 账号的建、删和凭据，以及 MCP 侧按人取 |
| 应用按人分数据（`webui.accounts: per_user`） | `plugin-spec.md:672-676`、`protocol/src/manifest/web.rs:83` | 把它从“分享提示”升成平台能兑现的约定 |
| 说话人请求头与轮次记录 | `plugin-spec.md:2456-2462`、`mcp/upstream.rs:216-235` | 递给插件的 `_meta`，群聊单人时可以代人 |
| 确认卡片、源会话、按人推送 | [ADR-0016](0016-mcp-bus-and-confirmations.md)、`mcp/audience.rs` | 卡片模板、结果卡 |
| 深链 `wonton://tab/plugin-path/<插件>/<base64url 路径>` | [ADR-0041](0041-app-links-and-shortcuts.md)、`protocol/src/link.rs:66-75` | 插件自己拼、核心校验；通知字段改名 `link` |
| 通知、按级别的推送类别 | [ADR-0047](0047-plugin-notices.md)、`gateway/notify.rs` | 收件人、`link` |
| 分享进来进聊天、Bot 存文件 | [ADR-0097](0097-share-in.md)、[ADR-0027](0027-bot-save-file.md) | 通向插件（`inbox`）和通向共享文件夹 |
| 结果文件按编号、按人授权 | [ADR-0020](0020-file-service.md)、`files/bot.rs:120-199` | 按房间可见 |
