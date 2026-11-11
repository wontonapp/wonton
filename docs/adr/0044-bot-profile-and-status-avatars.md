# ADR-0044 Bot 的名字、头像和状态头像：资料放在 Matrix，4 个状态由核心和 typing 给出，App 按状态播放

- 状态：已接受（2026-11-06）。状态收成 4 个、顶栏不写状态文字、默认素材由主人提供，都由主人定

## 背景

主人想让 Bot 像 Instagram 的好友一样能改名字和头像：主人在 App 里改，也能在聊天里对 Agent 说“把名字改成小云”，Agent
自己改。头像还要能随 Agent 的工作状态变化，支持动图和短视频（参照 muse bot）。

现状：

- Bot 的显示名和头像在安装时由核心按清单的 `bot` 设一次（插件规范的“Matrix Bot”），之后没有入口能改。核心每次安装都重设，
  保留数据重装、升级会把改过的名字冲掉。
- App 里 Bot 的头像是插件的图标，不读 Matrix 头像。时间线不显示资料变化。
- Matrix 每改一次 `displayname` 或 `avatar_url`，Bot 加入的每个房间都多一条 `m.room.member` 事件（[spec](https://spec.matrix.org/v1.16/client-server-api/#put_matrixclientv3profileuseridavatar_url)）。
  状态一分钟能变好几次，靠改 `avatar_url` 换状态头像会把房间的状态和时间线灌满。

参照（调研时的版本，细节来自第三方的分析，Meta 没有公开协议）：Meta 的 Muse 有 7 个状态（启动、待机、连接中、工作中、
做东西、完成里程碑、等子任务），素材是 5 段 MP4，连接中复用待机、等子任务复用工作中；用户只给一张静态图，服务端为每个
状态生成视频。Lobe UI 的 `AgentAvatar` 有 6 个状态，跟随系统的“减弱动态效果”。接 Claude Code hook 的桌宠都把状态并成
少数几个，并且单独有“等批准”。

## 决策

### 名字和静态头像：Matrix 的全局资料

名字和静态头像就是 Bot 账号的 `displayname`、`avatar_url`。所有聊天、所有 Matrix 客户端（包括 Element）看到的都是它，
聊天里由成员事件出一行“Hermes 改名为 小云”。

- 只有新建的 Bot 账号按清单的 `bot.display_name`、`bot.avatar` 设名字和静态头像。沿用旧账号时（保留数据重装、升级、安装失败
  后重试）核心不再碰它们。
- 时间线显示资料变化：名字变了是“旧名 改名为 新名”，头像变了是“某某 换了头像”。核心给 `TimelineContent::ProfileChange`
  （新名、旧名、头像变没变），文字在壳里（ADR-0043）。

### 状态头像：一组“状态 → 媒体”写在资料的自定义字段里

Bot 资料里多一个 [MSC4133](https://github.com/matrix-org/matrix-spec-proposals/pull/4133)（Matrix 1.16 的扩展资料）字段
`app.wonton.avatars`，一次声明一整组：

```json
{
  "app.wonton.avatars": {
    "states": {
      "idle":    {"url": "mxc://…", "mimetype": "video/mp4", "w": 480, "h": 480, "size": 812345, "duration_ms": 3000, "poster": "mxc://…"},
      "working": {"url": "mxc://…", "mimetype": "image/webp", "w": 512, "h": 512, "size": 402311, "duration_ms": 2000}
    }
  }
}
```

- 改这组不产生成员事件（Tuwunel v1.9.4 实测），所以换一整套也只是一次资料写入，状态切换完全不写 Matrix。
- 每一项是一个 Matrix 媒体（`mxc://`）。`poster` 是视频的静态首帧，可以不写，App 自己从视频取。
- 没写的状态用 `idle` 的；`idle` 也没有时用静态头像。App 不认识的键忽略，以后加状态旧 App 不受影响。
- 清掉全部状态头像时写成空表，不删字段（理由见“默认素材”）。
- 资料由 Bot 账号自己写。会说 Matrix 的 Agent 可以直接用自己的令牌写这个字段，不经核心；字段格式写进公开的插件规范。

### 状态：4 个，由核心看得到的事实加上 Matrix 的 typing 在客户端核心合成

状态只从下面这些能可靠拿到的事实里来，每一条对所有 Agent 都一样（ADR-0015），不看 Agent 自己报告什么：

| 事实 | 来源 | 对所有 Agent 都成立的原因 |
| --- | --- | --- |
| 插件在运行、健康检查过了 | 核心的插件状态和健康检查 | 核心管容器 |
| 有模型请求在进行 | 核心的模型网关入口（`models.wonton.internal`），按来源认出插件 | 经 magpie 的模型请求都过核心 |
| 有工具调用在进行 | 核心的 MCP 总线 | Agent 的 Wonton 工具都过总线 |
| 有确认卡片在等主人 | 核心的确认表（MCP 工具的卡片和 ACP 桥的 `/agent/permissions`） | 卡片都由核心发 |
| Bot 在它的私聊里“正在输入” | Matrix 的 `m.typing`，App 自己的同步里就有 | Hermes v0.22.5 每轮发（30 秒，轮末清掉），ACP 桥每轮发 |

客户端核心合成 `BotActivity`，壳只管按它显示：

| 状态 | 条件（从上往下取第一条成立的） |
| --- | --- |
| `offline` 离线 | 插件没在运行：在安装、升级、启动，停了或者出错 |
| `waiting` 等你确认 | 有这个 Bot 发起的确认卡片在等主人 |
| `working` 工作中 | 有模型请求或工具调用在进行，或 Bot 正在输入 |
| `idle` 待机 | 以上都不成立 |

- 不确定不是一个状态：App 没连上家里的服务器、或最近一次没取到时，`BotView::activity` 为空，只显示静态头像，不播任何状态
  头像（AGENTS.md 的“失败即停”）。
- 防抖：从 `working`、`waiting` 回到 `idle` 要先安静 2 秒，一个状态至少显示 1.5 秒；`offline` 立即生效。Agent 在两次模型
  请求之间跑自己的命令时，模型请求是断续的，没有防抖头像会来回跳。
- 没有“完成”“等子任务”：ACP 桥不声明子 Agent 能力，子 Agent 的工作只是根会话里的工具调用（ADR-0007 的验证结果），Hermes
  也不报告子任务，拿不到就不编。
- 和 Muse 的对应：启动、连接中 → `offline`；工作中、做东西 → `working`；待机 → `idle`。`waiting` 是主人要做的事，Muse 没有确认卡片。
- 状态是一个 Bot 的，不分房间：模型请求和工具调用核心分不出是为哪个房间。M1 的 Bot 只在自己的主房间里。
- 顶栏和列表都不写状态文字，只看头像。
- 这是 Bot 在不在线、忙不忙；一轮任务做到哪由 Agent 在时间线里发一条状态消息，见 [ADR-0082](0082-agent-run-status.md)，头像不读它。

### 状态怎么到 App：核心的长轮询接口

`GET /_wonton/api/bots/activity?after=<rev>`，只给经隧道的已配对设备。核心立即返回比 `after` 新的快照，没有就挂住最多 25 秒：

```json
{"rev": 42, "bots": [{"plugin_id": "com.nousresearch.hermes", "bot": "default", "user_id": "@hermes:…",
  "running": true, "model_requests": 1, "tool_calls": 0, "waiting": 0, "profile_rev": 3}]}
```

- 返回的是与请求者无关的事实（插件 ID、可选 Bot 名、计数和运行状态），合成在客户端核心。`plugin_id` 的线格式不变，
  `bot` 是增量字段；没给时是不知道 Bot 名，不能从用户 ID 猜出或补成 `default`，也不能据此开网页。
  网页访问权不放在运行事实里，按请求者判断的网页元数据见 [ADR-0045](0045-family-members.md#插件网页和工具)。
- `profile_rev` 在核心改过这个 Bot 的资料后加一，App 见到就重新取资料。Agent 直接经 Matrix 改的，App 在回到前台、
  打开聊天时重新取。Tuwunel 会经 Sliding Sync 的 MSC4262 扩展下发资料变化，但 matrix-sdk 0.19 的房间列表服务不带这个
  扩展；SDK 支持后改用它，`profile_rev` 随之删掉。
- App 只在前台、连着服务器时轮询；断了就把所有 Bot 的状态标成不确定。

### 主人在 App 里改

Bot 详情页（Agent 插件的详情页）加“资料”一节：头像、名字，和 4 个状态各自的头像。

1. 主人从相册选图片或视频。壳用平台能力处理：图片居中裁成方形、缩到 512 像素；视频取前 6 秒、居中裁方、缩到 480 像素、
   转成 H.264 的 MP4、去掉声音，并取首帧（iOS 用 AVFoundation，Android 用 Media3 Transformer 和 `MediaMetadataRetriever`）。
2. 客户端核心以主人的身份把文件传到 Matrix 的媒体库，拿到 `mxc://`。
3. 客户端核心调 `PUT /_wonton/api/bots/{plugin}/profile`（名字、静态头像、改哪几个状态、清掉哪几个）。核心下载回来检查（见
   “限制”），临时登录 Bot 的一台设备写资料，做完退出登录，和代主人加入房间的做法一样。App 不拿 Bot 的令牌。

### Agent 自己改：总线上的两个核心工具

- `wonton__get_profile`（只读）：自己的名字、有没有静态头像、哪些状态有头像，4 个状态分别什么时候显示，媒体的限制。
- `wonton__set_profile`：参数 `name`、`avatar`、`avatars`（状态 → 文件，`null` 清掉）。文件是 outbox 里的相对路径
  （和 `wonton__save_file`、写插件同一个 outbox），核心读出来、检查、以 Bot 的身份传到媒体库再写资料，返回新的资料。
  不收 `mxc://`：加密房间里的媒体是密文，Agent 要先解开存进 outbox。

权限：

- 只改调用方自己的 Bot：调用方由总线按来源地址认出（插件），核心按插件记录找到它的 Bot 账号。参数里没有“改谁”。
  没有 Bot 的插件调用时返回 `invalid_input`。
- 不发确认卡片：只改自己的资料，可以改回去，聊天里有成员事件的那一行，主人看得见。
- 频率：每个 Bot 每小时最多 10 次，超过返回 `rate_limited` 和还要等多久。主人在 App 里改不限。
- 名字：去掉首尾空白后 1 到 32 个字符，不能有换行和控制字符；不能叫核心账号的名字（Wonton、云吞、雲吞）或主人的名字，
  不分大小写，免得 Agent 被网页内容带偏后冒充核心或主人。
- 每次调用写审计。

### 限制

| 项 | 上限 |
| --- | --- |
| 格式 | 静态：PNG、JPEG、WebP。动图：GIF、动图 WebP。视频：MP4（H.264），声音忽略 |
| 尺寸 | 方形，边长 ≤ 640 像素 |
| 时长 | 动图和视频 ≤ 6 秒，循环播放 |
| 大小 | 每个 ≤ 2 MB |

核心按文件头认格式（不信扩展名和 `Content-Type`），读出宽高和时长再比上限；读不出来就拒绝，不猜。核心不转码，转码在壳里
（主人的路径）或由 Agent 自己做。上限同时写在 `wonton-protocol` 的常量里，三处共用一份。

### 默认素材：插件包里的一组，只补不覆盖

默认形象跟着 Agent 插件走，不打包进 App：清单的 `bot` 多一项 `avatars`，4 个状态各指向插件包里的一个文件。

```yaml
x-wonton:
  bot:
    display_name: { zh: Hermes, en: Hermes }
    avatar: avatar.png                 # 静态头像，也是 Element 等客户端看到的
    avatars:
      idle: avatars/idle.mp4
      working: avatars/working.mp4
      waiting: avatars/waiting.mp4
      offline: avatars/offline.mp4
```

- 核心安装、升级时，Bot 资料里还没有 `app.wonton.avatars` 字段，就按清单传上去、写进去；字段已经在（主人或 Agent 设过，
  包括清空成空表）就不碰。所以已经装着的 Hermes 升级到带素材的版本也会拿到默认形象，主人换过的不会被冲掉。
- 视频的首帧随文件放在旁边：`avatars/idle.jpg` 这样同名的 JPEG，有就一起传成 `poster`。
- 素材的规格就是上面的“限制”，另见下面给做素材的人的说明。插件包里单个文件的上限因此和头像一样（2 MiB），整个包的上限
  和上传的插件包压缩包一样（16 MiB），一套默认素材装得下；头像只传一次，App 按 `mxc` 缓存在本机，不用为省流量压小素材。素材到之前清单不写 `avatars`，App 显示静态头像。
- 为什么不打包进 App：Element 和别的客户端读的是 Matrix 资料，App 里的素材它们看不到；换一个 Agent 插件就是换一套形象，
  App 不用为某个 Agent 发版（ADR-0015）。

**给做素材的人：**

| 状态 | 什么时候出现 | 建议的样子 |
| --- | --- | --- |
| `idle` 待机 | 大部分时间 | 安静地待着，轻微的呼吸、眨眼；它也是别的状态缺素材时的替补 |
| `working` 工作中 | 在想、在查、在动手 | 明显在忙，节奏比待机快 |
| `waiting` 等你确认 | 有确认卡片等主人点 | 看向你、举手、等着，要一眼看出“轮到你了” |
| `offline` 离线 | 在启动、在升级、停了或出错 | 睡着、变暗、合上眼；不要吓人，启动中也是它 |

- 文件：每个状态一段，`idle.mp4`、`working.mp4`、`waiting.mp4`、`offline.mp4`，另配同名的首帧 `idle.jpg` 等；再给一张
  静态头像 `avatar.png`（就用待机的一帧）。
- 规格：方形，480×480（最大 640×640）；MP4，H.264，不要声音；每段 2–6 秒，首尾能无缝接上循环；每个文件不超过 2 MB
  （480 像素、4 秒的循环用 1 Mbps 左右就够）。首帧 JPEG 同样尺寸，质量 80 左右。
- 画面：主体居中，四周留出约 10% 的边，聊天列表里头像会被裁成圆形；背景用纯色或透明感的浅色，明暗两种主题下都看得清；
  头像在列表里只有 48 点大，动作要大一些、细节少一些。
- 也可以交 GIF 或动图 WebP，规格相同；同等画质下 MP4 最小，手机解码也最省电。

### App 怎么显示

- 聊天列表、聊天顶栏、Bot 详情页都显示 Bot 的 Matrix 名字和头像；Bot 没设头像时用它的插件图标。
- 有状态头像时按 `BotActivity` 播对应的那一个，切换时淡入淡出；没有状态头像、或状态不确定时显示静态头像。
- 省电省流量：
  - 视频静音、循环，只在 App 在前台、头像在屏幕上时播放；滑出屏幕、进后台就停，显示首帧。
  - 同一屏最多同时播 4 个，其余显示首帧。系统省电模式（iOS 低电量模式、Android 省电模式）和“减弱动态效果”打开时只显示首帧。
  - 媒体按 `mxc://` 缓存在本机，同一个只下一次；用到哪个状态才下载哪个，下完之前显示首帧或静态头像。

### 和插件规范的关系

公开的插件规范（wonton-docs）同步改：“Matrix Bot”一节写清名字和静态头像只在新账号上按清单设、`bot.avatars` 只补不覆盖、
`app.wonton.avatars` 的格式和 4 个状态、媒体的限制；“工具与 MCP 总线”加 `wonton__get_profile`、`wonton__set_profile`。

### Bot 换一整套（以后做）

主人说“换一套猫的头像”时，Agent 自己用模型网关的生图能力画出 4 个状态（同一个角色、不同的动作），写进 outbox 后调
`wonton__set_profile`。也可以像 Muse 那样从主人给的一张图出发，为每个状态生成一段。核心不生图，也不为生图加接口：
生图是模型网关和 Agent 的事。要做动图时，Agent 用自己容器里的工具把几帧拼成 GIF 或 MP4。

## 理由

- **资料放 Matrix，不放核心。** 名字和头像本来就是 Matrix 身份的一部分，Element 这类客户端也看得到；状态组和身份放在一起，
  Bot 换了核心也跟着走。会说 Matrix 的 Agent 不经核心也能自己改。
- **为什么不靠改 `avatar_url` 换状态。** 每次改都是每个房间一条成员事件，状态一分钟变好几次。
- **为什么状态不走 Matrix。** 试过的几条路都不合适：
  - Presence 的 `status_msg`：Simplified Sliding Sync（MSC4186）没有 presence 扩展，App 收不到。
  - 把当前状态也写进资料的自定义字段：不刷房间，但每次切换都要核心以 Bot 的身份写一次 Tuwunel 的数据库，核心还得一直
    拿着每个 Bot 的令牌；App 收到它又要等 SDK 支持 MSC4262。
  - 房间里的自定义状态事件：每次切换都进房间的状态和事件图，和成员事件一样灌房间。
  - 自定义的 ephemeral 事件：客户端接口不允许发任意 EDU（MSC2477 没进规范）。
  - 核心发 to-device：没有快照，App 刚启动不知道现在的状态；离线期间堆一串过期的状态。
  长轮询有快照、有增量，和 App 已有的“经隧道调核心接口”同一条路，没有新的传输。
- **为什么 typing 由 App 自己从 Matrix 拿。** 它是 Matrix 的标准，App 的同步里本来就有；用官方账号登录（不经 magpie）的
  Agent 核心看不到模型请求，只有 typing。
- **为什么只有 4 个状态。** 主人要简单；思考和动手对主人没有区别，合成一个“工作中”；启动中和离线都是“现在用不了”。
- **为什么 Agent 改资料不用确认。** 改名是主人在聊天里一句话就要见效的事，卡片会把它变成两步。风险是 Agent 被带偏后改成
  别的名字，用名字规则、频率上限和聊天里那一行改名记录兜住。
- **为什么核心不转码。** 转码要把 ffmpeg 带进核心镜像，核心要保持薄。主人的路径由手机转码（两端都有系统能力），Agent
  的路径由 Agent 在自己的容器里转。

## 代价与风险

- 家人等别的人看到的状态头像要他们的 App 也是 Wonton；Element 只看到静态头像和名字。
- Agent 不经核心直接改了 `app.wonton.avatars`，App 要到回前台、打开聊天才看到新的那组。
- 用官方账号、又不发 typing 的 Agent 只在用 Wonton 工具和等确认时显示得出忙。
- Agent 自己跑命令、上网搜索时核心看不到，只要 typing 还在就算工作中。

## 待验证

- 两端在聊天列表里同时播几个 480 像素的循环视频的耗电和发热（iPhone 和中档 Android）。
- Media3 Transformer 在 Android 10 上的裁剪和转码。

## 关联

ADR-0003（Matrix）、ADR-0007（Agent 与 Bot）、ADR-0015（标准接口）、ADR-0016（总线与确认）、ADR-0026（设计系统）、
ADR-0035（ACP 桥）、ADR-0043（多语言）。
