# ADR-0124 首页、Agent 的云朵形象与第一次出场

- 状态：已接受（2026-11-21）。首页、底栏、出场都是主人 2026-11-21 看着一段界面参考定的；云朵形象、颜色值加素材库、垫圆底由主人在同日定
- 补充：[ADR-0095](0095-no-tab-bar.md)（根页面、底栏）、[ADR-0026](0026-design-system.md)（Agent 身份色、App 图标）、[ADR-0044](0044-bot-profile-and-status-avatars.md)（头像由核心按素材合成，状态从 4 个加到 6 个）、[ADR-0016](0016-mcp-bus-and-confirmations.md)（首页的待确认卡片）

## 背景

主人 2026-11-21 给了一段界面参考，原话：“整体风格、布局充分学习视频这种最佳方案。”视频里是一屏只做一件事的首页：一句问候，
下面一叠主动卡片，底部一条大输入框；三项图标底栏；点 Agent 整屏变成饱和天蓝，一只立体角色弹跳落下、气泡说两句话，只在第一次
出现；Agent 聊天里它先开口、递出“连接 Gmail”的卡片；应用页两列卡片、长按浮起出“重新排序”“钉到底栏”。

同一天主人发了两张图定了角色：一朵浅天蓝毛绒云，头顶右侧一只白色螺旋独角、绕着两道棕色条纹，刺绣五官。识别点是云形、浅天蓝、
单边独角三样，App 图标改用它的原图抠像加蓝白渐变底（见“Agent 的云朵形象”）。第二张是同一风格的六个变体，变了身体颜色、头饰和表情。负责人 2026-11-21 补充：颜色不固定，
任意颜色值，六色只是预设；表情和头饰是可扩展的素材库；插件和 Agent 能往这个用户的素材库加素材；头像垫一个极浅的圆底。

只学视频的结构、节奏和手感，不搬它的名字、角色、文案和图标。

和已有决定的关系：[ADR-0095](0095-no-tab-bar.md) 去掉了底栏、把根页面定成聊天，[ADR-0017](0017-plugin-ecosystem-and-store.md) 的
“应用”网格是盖在聊天上的一层；[ADR-0044](0044-bot-profile-and-status-avatars.md) 定了 Bot 的名字、静态头像和 4 个状态头像，
头像素材由主人或 Agent 提供；App 没有首页，Bot 第一次打开和普通会话没有两样。

## 主界面：首页、Agent、应用三页，底栏只在三页上

- 主界面的根是三个平级的页：**首页**、**Agent**、**应用**。根页面从聊天换成首页，开口是选中的那一页。
- 底栏只在这三页上，三项固定加钉上去的应用，具体见 [ADR-0095](0095-no-tab-bar.md#主界面底栏2026-11-21-补充)。打开任何应用页
  （插件页、Bot 网页、公网网页、分享页）都是整页、没有底栏，左边缘右滑和 Android 返回键回去；返回落回选中那一页。
- 会话列表不是根页面，是 [ADR-0095](0095-no-tab-bar.md) 里那种整页：侧边栏“所有会话”一行和通知打开的是它，首页的未读卡片和
  `wonton://room/…` 直接打开那一间。聊天里返回回到来源页。
- 设置仍是主界面上唯一的整屏盖子，三页都进得去。配对页在没配对时占着根，和底栏无关。
- iPad 和宽屏按 [ADR-0103](0103-ipad-and-large-screens.md) 两栏：底栏画在左列底部，右列照旧是内容；应用页只铺右栏、不带底栏。

主人原话（定三页和底栏，见 [ADR-0095](0095-no-tab-bar.md) 的同日补充）：底栏只在主界面加回，选中项垫浅灰胶囊，图标下面不写字。

## 首页

- 一屏只有三样东西：上面的问候、中间一叠主动卡片、底部一条大输入框。没有页面标题，没有分区标签，内容垂直居中，四周大量留白。
- 问候是时段加名字再一句问句：核心给时段（早晨、下午、晚上、深夜）和主人的名字，句子在壳里按语言拼（[ADR-0043](0043-app-languages.md)）；
  还没取到名字时只问候，不编一个名字。
- 左上圆钮开侧边栏（[ADR-0095](0095-no-tab-bar.md) 的同一个侧边栏），右上圆钮起语音（用系统听写把话填进输入框，识别由壳做）。
- 卡片叠起来只露最上面一张的整张和下面一到两张的顶边，往上滑翻下一张，最后一张之后不再有牌；没有卡片时整叠不画。
- 输入框：左边“+”、中间占位字“问 Agent 点什么”、右边麦克风。回车或点发送把这句话发给默认 Agent（走已有的
  [`ChatAction::AskAgent`](../../crates/wonton-protocol/src/action_domains.rs)），核心接着切到 Agent 页，让人看见它。
  默认 Agent 没装或没选时输入框照常在，点了在输入框上方如实说“还没有默认 Agent”，给去设置的入口。
- 语音转出来的文字落在输入框里，人看过再发送；听写结果不确定时把原文和转换结果都亮出来，不直接发送。
- 卡片的内容、排序、上限全部由核心给，壳按类别画，不自己挑不自己排。卡片类别、来源、排序和空状态见“首页卡片的数据来源”。
- 除了上面这些，这页不加别的说明文字和图形；取卡片失败时是一条轻提示加重试，错误类别由核心给。

## 首页卡片的数据来源

卡片是 Agent 先开口的地方，每张卡都指回一个已经有来源的事，不新造事件流。

| 类别 | 数据从哪来 | 已有还是新做 | 谁的线 |
| --- | --- | --- | --- |
| 待确认 | 核心的确认接口 `GET /_wonton/api/confirmations?after=<版本>`（长轮询，版本变小当新列表），单张 `GET …/{id}`，回答 `POST /_wonton/confirmations/{id}`，停掉放行时段 `DELETE …/{id}/window`；规则见 [ADR-0016](0016-mcp-bus-and-confirmations.md)、[ADR-0024](0024-confirmation-scope.md)、[ADR-0045](0045-family-members.md)，代码在 `crates/wonton-core/src/gateway/confirmations.rs` | 核心有；客户端核心的 `ChatViewModel::pending_confirmations`（`PendingConfirmation`）已经带着标题、请求者、会话和到期时间，首页直接读它，点开复用现有的卡片详情 | 聊天与 Agent 线 |
| 未读 | `ChatViewModel::rooms` 每行的 `unread_count`（`RoomRow`），系统小组件的 `WidgetSnapshot::rooms` 同源 | 已有 | 聊天与 Agent 线 |
| 插件事件 | 已装插件的健康事实：插件停止、启动失败、页面出错（`TabState::Failed`、`RoomRow::plugin`）、有可升级版本（`PluginsViewModel::updatable`）；插件不经对话主动发的事件列为待对齐 | 事实已有，汇总成卡片是新做 | 插件底座线 |
| 晨报 | Hermes 的晨报，产出放哪、核心怎么取、已读状态都没定（主人 11-15 定的下一批） | 新做；晨报设计开工时两条线一起出 ADR，这张卡先留位，不用假数据 | 聊天与 Agent 线 |

- 排序：待确认最先（到期早的在前），然后未读（按会话最近活动），然后插件事件（出错、停止、可升级），最后晨报。同一件事只出一张卡：
  一个会话既有待确认又有未读时只出待确认那张，数量记在角标上。
- 上限 5 张，5 张以外的不进首页、不显示个数；这 5 张能一张张翻到底。总数为 0 时首页只剩问候和输入框，不垫骨架、不放示例卡。
- 卡片被处理掉才消失：确认答完、会话读完、插件问题修好、晨报读过。核心重新算，壳不做本地备忘。
- 连接断了、最近一次没取到时保留上次的卡片并标不确定；配对还没完成时不出卡片。

## Agent 页：默认 Agent 的私聊

Agent 页是主人和默认 Agent 的私聊，整页就是聊天时间线，底栏留着；它和从会话列表进来的同一个房间是同一份数据、同一个保活，
不是第二个聊天实例。

- 顶上是它的头像（默认表情，不随状态变）、名字；下面一行日期和一行灰字免责：“预览版可能出错，让它动手前先核对”。免责的类别
  由核心给、文字在壳里（[ADR-0043](0043-app-languages.md)）。
- 输入框占位是“给 <名字> 发消息”。Agent 回话前的三个点用 Matrix 的 typing，走已有的（[ADR-0044](0044-bot-profile-and-status-avatars.md)）。
- Agent 先开口：默认 Agent 的私聊建好的第一轮由核心以默认 Agent 的名义发一句自我介绍（内容由 Hermes 侧给，核心不编），
  之后开场白走已有的命令菜单卡片（[ADR-0054](0054-bot-command-menu.md)）。
- Agent 在聊天里递出的连接、安装、授权卡片全走已有的确认卡片（[ADR-0016](0016-mcp-bus-and-confirmations.md) 的
  `ConfirmationCard`：`ConfirmationCategory::ToolCall` 带所属插件和“连接”按钮），点完在片后的 `Succeeded` 结果卡上有“打开”“放到底栏”
  （ADR-0016 第 6 条）。不新造通道、不新造卡片类型；插件没装时 Agent 递的是安装的那张卡，装好再递连接。
- 房间没连上、插件在启动时照 ADR-0044 的分工显示状态，不把等待画成成功。

## 应用页

- 两列白色圆角卡片，左上名字、左下小图标，卡片之间大留白；顶上一行很淡的“长按可以钉到底栏”，右上齿轮进设置，
  最底一条搜索框加麦克风。
- 排在前面的是一组“我的应用”（顺序同上），用分隔留白和下面“全部应用”分开；下面列全部已装的。搜到的结果单独一屏。
- 长按一张卡：卡片浮起变亮、背景压暗，出系统 `contextMenu`，两项：“重新排序”“钉到底栏”（已钉的是“从底栏移除”）。
  长按菜单照 [ADR-0109](0109-chat-like-telegram.md) 用系统菜单，iOS 长按、Android 长按，同现有消息菜单。
- “重新排序”把当前这一组换成拖动模式，卡片右上出拖动柄，抖一下，拖完点空白处或“完成”退出；拖到最左边一张仍继续拖就移到另一组。
- 底栏直接按“我的应用”的顺序取钉着的那些，所以拖动改的是同一个顺序，应用页和底栏不会各排一次。
- 卡片点开就是整页，没有底栏。

## Agent 的云朵形象

主角色是一朵浅天蓝的毛绒云：头顶右侧一只白色螺旋独角、绕两道棕纹，刺绣风格的五官，底下两只圆脚。三个识别点到哪都要保住：
云形、浅天蓝、单边独角。

- **每个 Bot 一只云，长得不一样。** 身体颜色、头饰、默认表情由主人选；同一个人多个 Bot（管家、记账、健身教练）靠这三样区分。
  默认 Bot 拿到的是主角色：浅天蓝、白色螺旋独角、笑脸。
- **颜色是任意颜色值，不是固定几色。** 主人用取色器选 RGB，薄荷绿、薰衣草紫、蜜桃橙、樱花粉、奶油黄、浅天蓝是六个预设。
  核心拿身体模板按颜色值重着色：模板的明暗照留，色相和彩度取所选颜色（OKLCH 里换 L 和 C）。彩度太低时给一个最低值，
  不然会变成一块灰。
- **头饰七种，素材已有**：奶白色的螺旋角（两道棕色条纹，是默认）加卷曲条纹角、紫色条纹睡帽、闪电、白色绒球、黄色小独角、蓝色条纹独角；另有“不戴”。头饰只由人挑，不随状态变。
- **九个表情，素材已有（另有一套总览图）**：笑脸（默认，也是空闲）、思考、干活、困（睡着）、晕（出错）、吃惊（有事要你看）、笑哭、生气、吐舌斜眼。五个能给人挑的默认表情是笑脸、笑哭、吐舌斜眼、吃惊、生气（生气只给人挑，不用于状态）。
- **表情按状态换，颜色和头饰不动。** 表情和状态的对应见“状态头像”。
- **每人一张极浅的圆底**：从身体色提亮，画成直径等于画布的圆，合成时垫在最底下，不加描边。底色由核心算（身体色 OKLCH 的 L 提亮，
  C 降一半），不存。
- **角度。** Bot 头像、列表头像用正面（分层只对正面好对齐）。第一次出场的大图用主人原图那张 3/4 侧面，要不要改成正面见“待对齐”。头饰一律长在头顶右侧。
- **App 图标不重画。** 用主人原图抠出来的那一版（`design/app-icon/mascot/cutout.png`），保留四分之三侧脸、形状和神态，只垫蓝到白的渐变底；
  iOS 的浅色、深色、着色和 Android 自适应的前景、背景、单色图层都由它合成（2026-11-22 主人定），脚本是 `scripts/generate-app-icon.py`。
- 形象只出现在 Agent 的身份时刻：出场、头像、“Agent”那一项的选中图标；其余界面照 [ADR-0026](0026-design-system.md)，
  饱和天蓝只走 `agentIdentity` 令牌（见该篇同日补充）。

## 头像分层和素材库

头像拼三层：身体、表情、头饰。三层按同一张对齐模板和锚点表放，核心合成一张 512×512 的 PNG（[`image`](https://crates.io/crates/image)
crate，MIT 或 Apache-2.0，已在核心里），垫上圆底，以 Bot 的身份传进 Matrix 媒体库、写进 `avatar_url`。别的 Matrix 客户端看到的是同一张。

### 模板和锚点

- 仓库里放一张 1024×1024 透明底的正面灰白身体模板（无五官、无头饰），统一左上柔光、正下方接触阴影；五官和头饰的贴片都画在这套光照下。
- 锚点表是 JSON，按画布比例记脸框、眼线、嘴中心、头饰落点和角度、脚底线，发布时和模板放在一起。所有素材按这张表放，不靠模型去对齐。
- 贴片只收平视的刺绣五官；头饰根部画成插在绒毛里，合成时按落点放，核心在根部补一层很淡的接触阴影。

### 素材库存在哪

- **内置素材**编进核心的可执行文件（`crates` 下 `wonton-core/assets/cloud/`）：六色的预设值、九张表情、七个头饰；素材已有，带一张总览图。升级核心才换。
- **用户素材库**在服务器数据目录 `core/bot-assets/<用户 Matrix ID>/`，按人分：一个人挑给管家，不影响别人的记账 Bot。目录名去掉 `@`、`:` 换 `_`。
  人删掉的自定义素材一起删；素材被哪个 Bot 用着时先换回同类的内置素材再删。
- **插件和 Agent 加的素材**落进同一个用户素材库，走同一个导入入口（见“Agent 自己出素材”）；插件包里不另设素材格式。
- 素材进每个人的库都不改内置那批，内置的不删不覆盖。

### 身份和格式

- 素材 ID 带命名空间：`builtin:<键>`（如 `builtin:face/smile`）、`user:<uuid>`。插件和 Agent 加的一律是 `user:`，拿不到 `builtin:`
  也改不了别人的。
- 一个素材两个文件：`<uuid>.png` 和 `<uuid>.json`（`kind`、`anchors`、`name`、`author`）。格式只收透明底 PNG，1024×1024，
  边长误差不超过 1 像素，≤ 2 MiB；带 α 通道（整张不透明或全透明都拒）。
- 导入时核心按文件头认格式、量尺寸，锚点必须在画布里、关键点不许落在透明像素上；不合格原样返回 `invalid_input` 并说哪条没过。
  Agent 和插件给的素材先过一遍模板重合检查（身体是模板重着色，不校验）。
- 名字由导入方给，去空白后 1–24 字，不能和同类的内置名重名；库里同名的是两个素材。

## 状态头像：只换表情

`BotActivity` 从 4 个加到 6 个（[ADR-0044](0044-bot-profile-and-status-avatars.md) 的补充）：多出 `thinking` 和 `error`。核心本来就分得出模型请求、
工具调用和健康检查，够用了；头像从视频换成合成图以后，多两个状态的存储代价是两段 JSON 键。

| 状态 | 什么时候 | 表情 |
| --- | --- | --- |
| `idle` 待机 | 上面的都不成立 | 笑脸 |
| `thinking` 在想 | 有模型请求在途 | 眼睛往上看、嘴抿成小圆 |
| `working` 干活 | 有工具调用，或在打字 | 眯眼、专注 |
| `waiting` 有事要你看 | 有它发起的确认卡片等着 | 吃惊 |
| `error` 出错 | 插件启动失败、健康检查不过、这一轮运行异常结束 | 晕（X 眼吐舌） |
| `offline` 离线 | 插件在装、在升级、停了 | 困（半睁眼、打哈欠） |

上面已有的防抖、最短显示时长和“不确定时只显示静态头像”照 [ADR-0044](0044-bot-profile-and-status-avatars.md)，`error` 和 `offline` 立即生效。
错误和离线分得开：起不来给 `error`，主动停或者正在启动给 `offline`；分不出来的插件（老清单）一律 `offline`，不猜。

状态头像的写入：

- 静态头像和六张状态头像都是同一只云的 PNG，格式 `image/png`。静态头像用主人挑的默认表情，`avatar_url` 只指它；状态图进六个键。人挑的另两个备选表情不上传，只作换默认表情时的备选。这里不再放视频和动图；动图的最后一步等素材库稳定再做。
- 状态变一次合成一次，按（Bot，表情，颜色，头饰）缓存在核心，同一组合只合成一遍；一次状态切换最多上传一张新图。
- 上传换了 `avatar_url` 会刷成员事件，所以只有静态头像改动才写 `avatar_url`；六张状态图进 `app.wonton.avatars`（改它不写房间，ADR-0044 实测）。
- 老 App 不认识两个新键：一律忽略；它自己的 4 个状态里 `error` 和 `offline` 共用 `offline` 那张，不崩也不空白。

### 和 ADR-0044 “主人自己的图”怎么并存

Bot 资料页的头像一节改成两个来源，二选一：

1. **云朵**（默认）：颜色、头饰、默认表情，核心合成。
2. **自己的图**：照 ADR-0044 从相册选图或视频、按状态各放一个，核心不合成。

选自己的图清掉 `app.wonton.cloud`，选云朵清掉用户传的那组；主人自己的图片留在 Matrix 媒体库里（那组 `mxc`），切回来重新指回去就行，不另存一份。Agent 用 `wonton__set_profile` 改云朵字段和主人点选一样，不另走一条。

## 第一次出场

只在第一次点开 Agent 页时演一次。

- **触发**：默认 Agent 的私聊存在、连接着、这个人的“见过出场”没记下，且这一页刚从别处切进来。没装默认 Agent、没连上时顺延到条件成立的第一次。
- **标记存在服务器上**，跟着人走、跨设备：记在引导（`/_wonton/api/onboarding`）的数据里，用户换新手机再配对不再放一遍；
  引导本来也是每台服务器按人存（[ADR-0034](0034-onboarding-and-help.md)）。真正的动画进度不存，只存“见过”。
- **可跳过**：动画任一帧点屏幕就停在当前画面，气泡和按钮立刻出来；不额外加“跳过”按钮，点过和看到底一样记成见过。
- **演什么**：整屏渐变，上边 `agentIdentity`、下边白，3/4 大图从上方弹跳落下、落地一次眨眼，气泡依次说两句
  （先叫名字，再问“有什么能帮你分担的？”），下面是标题、两行说明和一个通栏大胶囊“继续”；点“继续”进普通聊天，气泡收起。
- **无障碍**：大图纯装饰，不读；气泡按顺序读，动画完把焦点放到标题；“减弱动态效果”打开时不做弹跳和眨眼，渐变压到 100ms 内
  （[ADR-0026](0026-design-system.md) 的动效规则），两气泡直接出现；静止时也保持对比度。
- 出场只挡 Agent 页那一层；人在动画中间切走，回来重演，见过标记在点过或播完时才写。

## 插件和 Agent 自己出素材

主人对记账 Bot 说“给它戴一顶厨师帽”，Agent 需要一个能落地的做法：

1. 主人在聊天里说，Agent 按模板和锚点表调图片模型（走模型网关，[ADR-0008](0008-model-gateway-magpie.md)），一次出透明底的顶着落点的帽子贴片，
   把成品和锚点 JSON 写进 outbox（和 `wonton__save_file` 同一个 outbox）。
2. 调总线上的 `wonton__add_avatar_asset`：`kind`（`face` 或 `headdress`）、`file`（PNG 的相对路径）、`anchors`（模板锚点）、`name`。
   核心校验后存进**这个人**的素材库，返回 `user:<uuid>`。
3. 用它：`wonton__set_profile` 的 `cloud.headdress` 写成返回的 ID。同一轮里做完，主人在聊天里就看到头饰换上了。

确认和权限：

- 加素材和改云朵不发确认卡片：只改自己的头像、可以换回去。改完整套（多个 Bot 一起）也不发。
- 只写调用方归属人的库，调用方由总线按来源认出（同 [ADR-0044](0044-bot-profile-and-status-avatars.md)）；没有 Bot 的插件返回 `invalid_input`。
- 每个 Bot 每小时最多 12 次导入、每次最多 3 个文件（主体 + 可能的泪滴、口水这类附件），超了返回 `rate_limited` 和还要等多久。
- 每次导入写审计，记来源（插件 ID、Bot）、素材 ID 和名字。
- 图片模型调不通、出图不合格时如实回报，不拿内置素材顶替。
- 插件也可以不经对话直接调这个工具；插件清单里不新增素材字段。

## 核心和壳的分工

厚核心、薄壳（[ADR-0005](0005-native-clients-thick-core.md)）：下面这些都住在客户端核心的 Rust 里，壳只渲染和把动作发回去。

- 首页卡片：算什么卡、怎么排、上限、去重、不确定的标记。
- 底栏的项、顺序、钉选状态；钉满时拒绝并给原因。
- 出场：条件、见过标记、气泡的两句话和按钮的先后。
- 云朵：可选的颜色板、表情、头饰清单（含素材库的读取），合成和上传；壳只在选色器里给 RGB。
- 输入框发给谁（默认 Agent），以及发送后切到 Agent 页。
- 文字只有类别和原始值；句子、按钮名、无障碍标签都在壳的本地化资源里（[ADR-0043](0043-app-languages.md)）。

### 接口草案

名字和类型是草案，下一张卡照它先提只改接口的 PR，再由实现 PR 落地。

`wonton-protocol` 的类型（改的是 `pages`、`home`、`bot_profile`、`onboarding` 几个模块）：

```rust
/// 主界面的三页。底栏三项固定是它们，钉上去的应用排在后面。
pub enum MainTab { Home, Agent, Apps }

/// 底栏的一项。
pub struct MainTabItem {
    pub entry: MainTabEntry,     // Home / Agent / Apps / Pinned(MyAppRef)
    pub label: String,           // 无障碍标签，底栏不写字
    pub icon: MainTabIcon,       // 固定图标，或插件图标
    pub badge: u32,              // Home：未读 + 待确认的总数；Agent：默认 Agent 私聊的未读；Apps：出错 + 可升级。0 不画
}
pub enum MainTabEntry { Home, Agent, Apps, Pinned(MyAppRef) }
pub enum MainTabIcon { Home, Agent, Apps, Plugin(PluginIcon) }

pub struct MainTabsViewModel {
    pub tabs: Vec<MainTabItem>,
    pub selected: MainTab,
    pub pin_cap: u32,            // 3
    pub pin_full: bool,          // 钉满提示，下一次成功钉上时清
}
```

- `PagesViewModel`：`root: TabRef` 换成 `tab: MainTab`，删 `shows_apps`；`stack` 只管应用页，压着页面时底栏不画。
- `PageAction` 增 `SelectMainTab { tab: MainTab }`、`PinToBar { app: MyAppRef }`、`UnpinFromBar { app: MyAppRef }`；
  `AddMyApp` 加 `pinned: bool`（同时钉上）；`MoveMyApp` 不改，底栏跟着走。钉满时 `PinToBar` 返回 `invalid_input` 带原因。
- `MyAppEntry` 加 `pinned: bool`，存进 `app.wonton.layout` 的同一条。
- `TabRef` 只留页面（`Plugin`、`PluginPath`、`Bot`、`Url`、`UiResource`、`SharedApp`、`Widget`）；`Chat` 换成 `Rooms`（会话列表页，侧边栏
  “所有会话”和通知用），删 `Apps`。`AppLink` 增 `MainTab { tab: MainTab }`，链接是 `wonton://tab/home`、`/tab/agent`、`/tab/apps`，
  旧的 `tab/chat` 不留；`AppLink::Tab` 仍指页面，会话列表另有 `wonton://rooms`。设置仍走 `wonton://settings`。
  我的应用仍只收插件和页面（`MyAppRef::Plugin` / `Page { tab }`），三个根页不占钉位。

```rust
/// 首页。
pub struct HomeFeedViewModel {
    pub greeting: Option<HomeGreeting>,
    pub cards: Vec<HomeCard>,
    pub uncertain: bool,          // 断线、最近没取到：卡片照留，壳标不确定
}
pub struct HomeGreeting { pub part: DayPart, pub name: Option<String> }
pub enum DayPart { Morning, Afternoon, Evening, Night }
pub struct HomeCard {
    pub id: HomeCardId,
    pub kind: HomeCardKind,       // Confirmation / Unread / PluginEvent / Digest
    pub title: String,            // 服务器给的原文，或会话名、插件名
    pub detail: Option<String>,
    pub icon: Option<PluginIcon>,
    pub badge: Option<u32>,
    pub expires_at_ms: Option<u64>, // 确认卡片
}
pub enum HomeCardId {
    Confirmation { id: String },
    Room { room_id: String },
    Plugin { plugin_id: String, kind: PluginEventKind },
    Digest { message_id: String },
}
pub enum PluginEventKind { Failed, Stopped, Updatable }
pub enum HomeAction {
    Ask { text: String },          // 发给默认 Agent，然后切到 Agent 页
    OpenCard { id: HomeCardId },   // 卡片的处置全在核心（进房间、开确认详情、开插件页）
    Refresh,
}
```

```rust
/// Bot 的云朵。两个 ID 都用素材库的命名空间串。
pub struct BotCloud {
    pub body: String,              // "#RRGGBB"，任意值
    pub face: String,              // "builtin:face/smile"
    pub headdress: Option<String>, // "user:<uuid>"，不戴时 None
}
pub enum CloudPart { Body(String), Face(String), Headdress(Option<String>) }

// BotAction 里增：
SetBotCloud { plugin_id: String, cloud: BotCloud },
SetBotCloudPart { plugin_id: String, part: CloudPart },
// SetBotAvatar / ClearBotAvatar 保留，是“自己的图”那个来源。

/// 挑选界面的素材一项。builtin 和 user 同列，壳按 author 分组。
pub struct CloudAsset {
    pub id: String,
    pub kind: CloudAssetKind,      // BodyPreset / Face / Headdress
    pub name: String,
    pub author: CloudAssetAuthor,  // Builtin / Plugin { plugin_id } / User
    pub color: Option<String>,     // 只对 BodyPreset：预设的 "#RRGGBB"，点它就是 SetBotCloudPart::Body
    pub preview: AvatarImage,
}
// BotView 里增 cloud: Option<BotCloud>、custom_media: bool、face: Option<String>（当前该播的表情 ID）
```

```rust
// OnboardingViewModel 里增；Onboarding 的服务器数据里存 agent_intro: bool。
pub struct AgentIntroView {
    pub show: bool,               // 该演：条件都满足且没见过
    pub name: String,             // 默认 Agent 的名字，气泡里叫主人
}
// OnboardingAction 增 FinishAgentIntro：点过或播完都发，幂等。
```

服务器接口：

- `GET /_wonton/api/cloud-assets?plugin=<插件>`：内置、这个人的、这个插件给的素材清单；`GET` 单项带原图。
- `POST /_wonton/api/cloud-assets`：主人从相册给自己的库加素材（Owner 令牌）。
- `DELETE …/{id}`：只删人自己库里的。
- `GET /_wonton/api/home`：首页卡片快照；推送沿用确认和未读各自的现有通道，首页不另开长轮询。
- 总线工具 `wonton__add_avatar_asset`，参数见“插件和 Agent 自己出素材”；`wonton__set_profile` 加 `cloud`。

服务器的 Bot 资料写入（[ADR-0044](0044-bot-profile-and-status-avatars.md) 的 `PUT /_wonton/api/bots/{plugin}/profile`）由核心侧自己接，
接口改动由本线提的卡带着走；Bot 资料和 Bot 网页那一路没有这个卡，人的头像在 #874。

## 理由

- **首页是 Agent 先开口的地方。** 主人要的是“它替我盯着”，卡片全是有来源的真事（确认、未读、插件坏没坏），不是推荐流；
  空的时候就是一屏留白加一个输入框，不占地。
- **图标用原图抠像，不重画。** 角色本身就是识别点，重画一版只会丢神态；抠像已经备好，渐变底只负责让它在桌面上立得起来，
  形状、角度、神态一个不动。
- **卡片类别由核心算。** 排序、去重、上限、不确定的标记写在 Rust 里，两端一样，也能写行为测试；壳拿到类别只画。
- **根页面从聊天换成首页。** 聊天是过程，首页是“现在要我做什么”。会话列表仍在，只是挪进侧边栏一行，不再占根。
- **底栏只在三页。** 应用页照旧决定整页（ADR-0095 的理由没变：自带底栏的网页不和我们的栏叠两行）；主界面三页给得回一眼能点的入口。
- **形象是云，颜色值加素材库。** 固定六色看着够，但“给我的记账 Bot 戴厨师帽”这种要求只能靠素材和任意色值；三层素材分开，
  换一样不动另两样，Agent 也能自己补素材。
- **由核心合成。** Matrix 资料里的头像必须是真图，别的客户端才看得见（ADR-0044）；三层叠图的自绘逻辑写两遍迟早不一致，
  素材校验也只有一处。
- **一出场只一次、可跳过、跟随减弱动态。** 出场是介绍不是每次的封面；看过的人有跳过和减动两条路，不挡正事。

## 没选的方案

- **首页就做成聊天列表。** 现在的根就是列表，改动最小。没选：列表只说得清“和谁说过话”，说不清“哪件事等我点头”；
  待确认、插件坏了要靠翻会话和看角标，Agent 也没地方先说一句。
- **底栏项带文字。** 更好认，第一次用不必猜图标。没选：主人定的是三项图标加胶囊，带文字要占两行高度，钉满以后一行挤不下；
  图标加读屏标签已经够认。
- **出场每次都放。** 有仪式感。没选：每天回来都等一段弹跳才摸得到输入框，跳过按钮也成了固定装饰；一次见过就够。
- **固定六色、不给任意值。** 素材和校验都简单，调色只要一张表。没选：主人明确要取色器；任意值在核心重着色，和六色同一套代码，
  代价只是多一个校验。
- **插件清单里声明云朵素材。** 和插件默认头像同一处。没选：插件卸载就得抽掉素材，同一个插件给两个用户改过的素材也分不开；
  导入进用户库以后和来源插件解耦，卸载不动已选的。
- **出场标记只存这台设备。** 不用动服务器数据。没选：引导本来按人存在服务器上，换台手机再看一遍同一段介绍是重复；跨设备少一次的代价是新手机不再看到，
  而 Agent 页的图标摆在那儿。

## 代价与风险

- 首页要多一次取卡片的请求和一次推送处理，断线时得把“不确定”画得出来，不能和“没事”混；空状态和不确定的区分要在两端各验收一次。
- 卡片叠只露上面一张，第三张往后要滑才知道有；5 张的上限挡住了更多，但顺序由核心定，用户没法调。
- 根页面换了、`shows_apps` 没了，`MainNavigationUITests`、`SidebarUITests` 和一串快照都要跟着改；老装置不读新数据，第一版不做迁移。
- 出场那段动画是自绘的渐变加位移，减弱动态、大字号、窄屏三种情况要各看一次，动效多了容易掉帧。
- 合成一张 512 的 PNG 加两次上传要几十毫秒到几百毫秒，状态切换快时要靠防抖和缓存压住；低端机上要实测。
- 头像在别的客户端只是一张 PNG，动不了；等人换回视频素材才有动作，而视频路径和云朵是二选一。
- 任意颜色都能选，深色、灰、荧光都要靠最低彩度和圆底救，得有一组边界颜色的合成截图看。
- 素材是用户内容：一张 1024 的 PNG 每个 ≤ 2 MiB，没有总容量上限，人会攒很多；先按人限 200 个，到了让先删。

## 待对齐

- **插件底座线**：插件不经对话主动上报事件（进展、要人看一眼的事）目前没有通道；首页的“插件事件”先用健康事实，
  要真事件就缺一个总线上的上报工具。问：这条通道做不做、挂在哪、限什么频。
- **聊天与 Agent 线**：首页的未读和待确认直接读 `ChatViewModel` 的字段，不另开接口；确认卡片点开进详情是壳的现有页面，
  首页只给 `HomeCardId`。问：`ChatViewModel` 这两个字段的稳定承诺要不要写进契约测试。
- **聊天与 Agent 线**：晨报的产出、取法和已读都没有，等晨报设计开工时一起出 ADR；这页只留 `Digest` 类别。
  问：晨报 ADR 什么时候开、在那之前首页要不要先出这一类。
- **Hermes 默认 Agent**：出场的第一句自我介绍由 Agent 自己发还是核心代发（现在草案是 Hermes 侧给文本、核心发），
  以及默认 Agent 的私聊建没建由谁先动。问：Hermes 侧接不接这句。
- **Bot 资料和 Bot 网页那一路**：云朵编辑器就长在 #934 收过的那节头像里，替换现在“选一张图”的按钮，#798 的工具和 #874 人的头像不动。
  `BotActivity` 加两个值、`app.wonton.avatars` 加两个键、`BotView` 加字段都在协议里，他们负责的 Bot 资料写入和 web 端要跟着认；
  接口 PR 出来后请他们先看。问：资料页和 web 端谁先改、接口 PR 要不要等他们一起。
- **设计**：素材（身体、九张表情、七种头饰、锚点表）已备，总览图在；素材命名由设计补。App 图标已按原图抠像加蓝白渐变底生成。问：出场大图用 3/4 还是正面、动画稿什么时候能看。

## 待验证

- 合成一张 512 PNG 在 iPhone 12 一档的低端机上的耗时；连切状态时上传和成员事件的实际条数。
- 首页卡片在断线、切账号、插件卸载时的去留，两端各走一遍。
- 出场在减弱动态、最大辅助字号、深色和窄屏上的样子；跳过手势会不会和左边缘返回打架。
- 两列卡片在大字号下的换行和长按菜单的位置。

## 关联

[ADR-0005](0005-native-clients-thick-core.md)、[ADR-0016](0016-mcp-bus-and-confirmations.md)、[ADR-0024](0024-confirmation-scope.md)、
[ADR-0026](0026-design-system.md)、[ADR-0029](0029-platform-material.md)、[ADR-0034](0034-onboarding-and-help.md)、
[ADR-0041](0041-app-links-and-shortcuts.md)、[ADR-0043](0043-app-languages.md)、[ADR-0044](0044-bot-profile-and-status-avatars.md)、
[ADR-0054](0054-bot-command-menu.md)、[ADR-0095](0095-no-tab-bar.md)、[ADR-0100](0100-system-entry.md)、
[ADR-0103](0103-ipad-and-large-screens.md)、[ADR-0108](0108-hermes-default-kernel.md)、[ADR-0109](0109-chat-like-telegram.md)
