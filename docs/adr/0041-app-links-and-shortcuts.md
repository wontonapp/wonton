# ADR-0041 App 链接、快捷指令和桌面快捷方式

- 状态：已接受（2026-11-06）。“问 Bot”直接发出去由主人确认

## 背景

主人想不打开 App 就能做几件常做的事：用 Siri 或快捷指令问默认 Agent 一句话，从桌面长按图标直接新建会话、
打开某个插件页。两端的系统能力不同：iOS 有 App Intents（Siri、快捷指令、Spotlight 都用它），Android 有
App Shortcuts（长按图标的菜单、固定到桌面）。两端都要一个能写进系统、以后还能认出来的“打开 App 里某一处”的写法。

## 决策

### 链接的格式由核心定

App 里能直接打开的地方是 `AppLink`，写成 `wonton://` 链接。编和解都在核心（`app_link_url`、`parse_app_link`），两端不自己拼：

| 链接 | 打开 |
| --- | --- |
| `wonton://tab/chat`、`/tab/apps`、`/tab/settings` | 底栏的三个固定页 |
| `wonton://tab/plugin/<插件>`、`/tab/plugin/<插件>/<页面>` | 插件页；没固定在底栏上的盖在主界面上打开 |
| `wonton://new-chat` | 发起私聊 |
| `wonton://agent-chat` | 和默认 Agent 的聊天 |

认不出的链接（新版 App 写进系统、旧版读到的，或者被改过的）照常打开 App，不报错。插件和页面的 ID 只认清单里的
短名写法，`..` 这类写法不认。配对码 `wonton://pair?…` 是另一种东西，不走这里。

### “问 Bot”直接发给默认 Agent

带一句话的“问 Bot”用 `Action::AskAgent`：核心把这句话发进和默认 Agent 的私聊，私聊还没有就先建，Bot 发起的邀请
还没接受就先接受；壳随后按 `OnboardingViewModel::primary_bot_user_id` 打开这个私聊。没有默认 Agent 时如实拒绝
（`NotReady`），不把这句话悄悄丢掉。

为什么直接发、不只是填进输入框：用 Siri 和快捷指令的人要的就是一句话发出去。插件网页的 `ui/message` 只填草稿
（[ADR-0033](0033-plugin-tabs.md)），是因为那是网页替主人说话；这里是主人自己说的。

### 两端各用系统的方式

- iOS：App Intents。“问 Bot”带一个文字参数；“打开页面”的候选来自底栏上的项和可以添加的页面；“新会话”。
  `AppShortcutsProvider` 给 Siri 和 Spotlight 的短语。Intent 在 App 里运行（核心在 App 进程里）。
- Android：静态快捷方式“新会话”“问 Bot”（打开和默认 Agent 的聊天），动态快捷方式是固定在底栏上的插件页，用插件
  自己的图标。都经 `wonton://` 链接进 App。

## 代价

- iOS 的 Intent 要把 App 拉起来才能用核心，“问 Bot”不能在锁屏上无声完成。
- Android 的快捷方式不带文字参数，“问 Bot”只能打开聊天，要直接发一句话得用 iOS 或者在 App 里打字。
