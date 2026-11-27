# ADR-0130 去掉重点色：界面只有黑白灰，靠灰阶和投影分层

- 状态：已接受（2026-11-27）
- 取代：[ADR-0026](0026-design-system.md) “颜色”一节里重点色的令牌和推导、“纸质磨砂”里按重点色算的对比度；[ADR-0028](0028-settings-page.md) 的“重点色”一项；
  [ADR-0109](0109-chat-like-telegram.md) 第 9 节里用重点色的那几项（出气泡、选项按钮、名字、跳转高亮、聊天背景）

## 背景

ADR-0026 的界面是黑白为主加一个用户可换的重点色，默认 `#FEEBEA`，核心按它推导五个令牌（`accentFill`、`accentInk`、`accentInkOnFill`、`textOnAccentFill`、`accentFillBorder`）。主人 2026-11-27 看过以后定：去掉主题色，先只做黑白加阴影调节，分浅色和深色两种；深浅两种模式下图标颜色要协调。

浅色的默认重点色对白色画布只有 1.15:1，选中、自己的气泡、角标都要靠描边令牌和换字色才分得出来；用户还能选任意颜色，推导和颜色网格测试是为此存在的。

## 决策

### 颜色

两套固定的令牌，写在客户端核心（`crates/wonton-client/src/appearance/themes.rs`），经 `AppearanceViewModel.light` / `.dark` 给两端。没有用户可选的颜色，也没有推导：

| 令牌 | 浅色 | 深色 | 用在 |
| --- | --- | --- | --- |
| `canvas` | `#FFFFFF` | `#0E0E0E` | 页面底 |
| `surface` | `#FFFFFF` | `#191919` | 卡片、别人的气泡（浅色） |
| `surfaceSubtle` | `#F4F4F4` | `#262626` | 分组底、输入框、聊天背景（浅色）、别人的气泡（深色） |
| `textPrimary` | `#111111` | `#F5F5F5` | 正文 |
| `textSecondary` | `#5E5E5E` | `#ABABAB` | 说明、时间 |
| `separator` | `#E4E4E4` | `#333333` | 分隔线 |
| `primary` | `#111111` | `#454545` | 强调填充：主按钮、选中的行、待确认提示条、角标、自己的气泡、占位图的底 |
| `onPrimary` | `#FFFFFF` | `#FFFFFF` | `primary` 底上的字和图标 |
| `success` / `warning` / `danger` | `#18753C` / `#8A4B00` / `#B42318` | `#7CDCA0` / `#FFC379` / `#FFA69F` | 状态，不算主题色 |

原来用重点色的地方分两种换：大块的填充用 `primary`，上面的字和图标用 `onPrimary`；页面上的强调字和图标用 `textPrimary`。浅色的填充是黑，深色的填充是比卡片浅一档的深灰，上面都是白字（测试断言 `onPrimary` 对 `primary` ≥ 4.5:1，`primary` 对三种页面底色 ≥ 1.5:1，深灰和卡片之间的层次主要靠投影）。开关的“开”、勾选圈、提示小圆点这类小记号面积太小，深灰在深色底上看不出来，所以用 `textPrimary` 填、`canvas` 画里面的勾和滑块；浅色下这两档和 `primary`、`onPrimary` 同值，看起来不变。`accentFillBorder` 跟着删掉。和正文同色的强调靠形状和字重区分：链接带下划线，选中的图标换 Fill，群聊里的名字是半粗。

Android 的 Material 主题里 `primary` 是字色（TextButton、进度圈、光标都取它），所以 Material 的 `primary` 对应 `textPrimary`，我们的 `primary` 只给 `primaryContainer`。

噪点照 ADR-0026 留在强调色的填充上（`emphasisFill`），气泡不叠，和原来一样。

### 分层

灰阶分出页面、分组和气泡的底；同色的底上再靠投影浮起一层。投影是黑色，浓度和形状是 `design/tokens.toml` 的令牌（`size.shadowBlur`、`size.shadowY`、`ratio.shadow`、`ratio.shadowDark`）：深色底上同样浓度看不出来，所以深色另取一档。用在聊天气泡上，iOS 消息下的选项按钮也用。

聊天背景不再盖一层重点色（删掉 `ratio.wallTint`），只是灰阶的底加 `textPrimary` 的细线图案（`ratio.wallPattern`）。跳转高亮和附件面板当前一项的底改用 `textPrimary` 的低浓度。

### 设置和存储

外观页去掉重点色一组（色板、“更多”、十六进制输入）；`SetAccentColor`、`parse_accent()`、`accent_hex()` 和 `AppearanceViewModel` 里的 `accent`、`presets`、`more_colors`、`swatch_columns` 一并删掉。`appearance.json` 里的其余几项照旧；旧文件里的 `accent` 字段读的时候忽略，下次改设置时文件重写就没有了，不写迁移。

### 插件网页

插件规范承诺过 `--wonton-accent-fill`、`--wonton-accent-ink`、`--wonton-accent-ink-on-fill`，名字不改，值换成黑白灰：`fill` 是 `primary`，`ink` 是 `textPrimary`，`ink-on-fill` 是 `onPrimary`。按规范写的页面（底用 `fill`、底上的字用 `ink-on-fill`、页面上的字用 `ink`）照样读得清；把 `ink` 写在 `fill` 上的页面浅色下会变成同色，所以插件指南的样板改成按钮上用 `ink-on-fill`。

界面改黑白后，Agent 身份色（ADR-0026 的 `agentIdentity` / `agentInk` / `agentBacking`）照旧保留，是黑白界面里除图标和状态色外唯一的颜色，只用在 ADR-0026 列的那几处。本篇不动它的令牌和实现。

## 理由

- **只留一档强调：** 主人要的就是黑白。重点色是浅色的纸色时，填充、字、描边要分三四档才读得清；改成黑白灰以后只剩填充（`primary`）和字色（`textPrimary`）两档，壳少了一半的分支。
- **固定值放核心而不是 `tokens.toml`：** 颜色仍按明暗成套给，壳的取法不变（ADR-0026 的“状态在核心”），插件网页的变量也由核心按同一份写。
- **深色的强调填充是深灰而不是白：** 白底黑字和浅色的黑底白字对称，但主人看过深色截图，大块的白（自己的气泡、选中的行、提示条）夜里太刺眼。深灰比卡片浅一档，上面白字，和卡片的层次交给投影。小记号仍用白，免得在深色底上看不出。
- **变量名不改：** 规范是公开的，已有插件在用；只换值不会让页面报错，最坏是按规范之外的写法同色。

## 代价

- 深色模式里强调填充和卡片只差一档灰，选中的行、主按钮比浅色里弱，靠投影和字重补。
- 强调和正文同色，链接、名字这些只能靠下划线和字重区分；以后新加的强调文字也要带一样形状上的区别，不能只换颜色。
- 把 `--wonton-accent-ink` 当按钮字色的插件页面，浅色下按钮会变成同色，要按规范改成 `ink-on-fill`。

## 待验证

- 两端深色模式下深灰气泡和别人的气泡（`surfaceSubtle`）在长聊天里分不分得清，真机看一次。
- 投影在长列表滚动时的帧率，两端低端设备各测一次。
- 彩色图标（iOS 的 Kubai）放在黑、深灰两种强调底上的协调度，和图标那边一起看。
