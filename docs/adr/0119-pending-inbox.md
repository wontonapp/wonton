# ADR-0119 收件箱：要用户决定和要用户知道的事，用五种条目进核心里的一页

- 状态：已接受（2026-11-25 主人拍板：20 个问题全部按推荐；第 6 题“页面能不能跑脚本/联网”按 23:4x 补定：脚本可以跑，联网只按发送方声明的域名名单放行，卡上写明“会访问：xxx”，不写名单就完全断网。范围是收件箱，不做待办；条目要能承载选择卡、HTML 报告、收尾框等富元素。上一版“通知/待办”两段式作废。2026-11-26 补 `confirm` 的 `comment`/`requires_comment`（[ADR-0123](0123-hubs-and-managed-devices.md)）。）
- 取代：[ADR-0016](0016-mcp-bus-and-confirmations.md) 第 2 步里的“等你确认”列表（`GET /_wonton/api/confirmations` 的列表、聊天列表里的入口和它的角标）。确认卡片本身、来源会话、回答接口都不变，只是列表换成收件箱，确认变成其中一种 `confirm` 条目
- 部分取代：[ADR-0038](0038-multi-step-cards.md) 的“问题卡片 M1 不做”。问题卡片作为 `choice` 条目进收件箱；`StepCard` 的形状、上限和答案词汇直接沿用，不再只在 Bot 聊天里出现
- 修订：[ADR-0047](0047-plugin-notices.md)（插件通知不再发进 Wonton 私聊，改成 `notice` 条目）、[ADR-0056](0056-internal-mailbox.md)（应用来信同上）、[ADR-0045](0045-family-members.md)（文件链接、家人加入的通知同上）、[ADR-0050](0050-cross-server-chat.md)（配对请求同上）、[ADR-0101](0101-notifications.md) 第 3 节（应用角标的数里加上待回答的条目，算法按下文第 9 节改）
- 关联：[ADR-0003](0003-matrix-and-encrypted-push.md)（推送链路）、[ADR-0118](0118-im-three-layers-push-with-body.md)（推送带正文、房间不再端到端加密；分支待合）、[ADR-0117](0117-bot-task-live-progress.md)（Bot 长任务进度）、[ADR-0082](0082-agent-run-status.md)（运行状态）、[ADR-0052](0052-chat-pull-panel.md)（MCP Apps 宿主、页面沙箱）、[ADR-0088](0088-family-accounts.md)（管理员卡片）、[ADR-0102](0102-app-lock.md)（App 锁）、[ADR-0026](0026-design-system.md)/[ADR-0029](0029-platform-material.md)（外观）、[ADR-0005](0005-native-clients-thick-core.md)（厚核心，壳只渲染）
- 实施拆分：协议 → 核心表和安全宿主 → 两端壳 → Agent 工具 → Hermes/worker 桥 → 拆私聊，每步的范围和验证见任务结果文件 `~/wonton-dev/tasks/inbox-design.result.md`。

## 用户看到什么

打开云吞，聊天列表最上面固定一行“收件箱”，右边的数字是有几件事等你。点进去是一列卡片：机器人问你一个问题（每题几个选项，推荐项排第一，最后永远有“其他”），选完点“提交”；一份报告或网页，卡片上一句说明，点开在 App 里看，看完在页面底下点“通过 / 退回”；一条两三句的收尾，最后问一句要不要继续；还有装插件的确认、家人加入、下载完成的通知。答完的卡片写“已回答”，一天后进“已处理”，三十天后归档。原来那个“云吞”私聊不再塞通知，旧消息从页面最下面翻回去。

收件箱里只有五种东西（都不是任务）：

| 类型 | 用户看到什么 | 用户做什么 |
| --- | --- | --- |
| `choice` | 一张卡，1–4 个问题；每题 2–4 个选项（或 0 个，直接自由回答），推荐项带“推荐”，最后有“其他”；每题可写备注 | 选、写、提交，或整张不回答 |
| `page` | 一句说明 + 一个 HTML 页面或文件；点开在 App 里读 | 读，点页面底下的 0–3 个动作（通过、退回、打开等），或“知道了” |
| `recap` | 两三句“现在是什么状态、还缺什么”，至多一个问题、1–2 个按钮 | 点按钮或“知道了” |
| `confirm` | 熟悉的确认卡片（装插件、工具放行、受控发送、配对、家人加入） | 允许、拒绝、允许一段时间 |
| `notice` | 一条只读通知（下载完成、备份失败、验证码信、磁盘将满） | 知道了、打开链接 |

“收件箱”不是待办清单：没有 assignee、没有 done、没有项目。一条条目要么等你回答，要么等你读。Bot 要你“做点什么”时，发一张 `recap` 或 `page` 把要做的下一步写清楚，做完是 Bot 再发一条，不是用户勾任务。

## 背景

主人 2026-11-25 22:5x 的原话：“就是收件箱，但是这个收件箱能收的元素不只是一个只可以读的消息而已，而是有类似于你给我弹的这个卡片这种可以选，然后甚至是可以内置一个 HTML 的链接这类的比较丰富的一个元素，它的设计的主要目的是类似于我们现在在开发云吞这个项目可以通过一些其他的方式节省用户的精力，而不是让用户自己去翻聊天记录这种非常低效率的沟通方式。”

这句话把范围从上一版的“通知 + 待办”收回到“收件箱”，但把**条目的丰富度**提上来了。参照物不是邮件，是协调会话现在和主人沟通的方式（Claude Code 桌面/手机端）：

- `AskUserQuestion`：一张卡 1–4 个问题，每题 2–4 个选项，推荐项第一，永远有“其他”，每题可备注，选项可带预览；
- `SendUserFile`：一份 HTML 或文件，带一句说明，`display: render` 在 App 内渲染，`attach` 当附件；
- `wrap_up`：收尾框，两三句状态 + 至多一个问题 + 1–2 个按钮。

这些问题和文件现在只能留在聊天记录里：主人要往上翻、找上下文、把答案打回聊天；Bot 也没法把“问题、选项、备注、报告、下一步按钮”结构化地交出去。收件箱要把这件事产品化：**发起方把要主人看或拍板的东西做成结构化条目推进来，主人就地处理，答案结构化地回到发起方。**

已经有的地基，不用新造：

- 条目在核心里、由核心给 ViewModel、壳只渲染（AGENTS.md“厚核心薄壳”）。
- 推送、长轮询、按人过滤、到期、已回答/已过期、管理员谁先答算谁的（ADR-0016/0088）。
- 问题卡片的形状、上限、答案词汇（ADR-0038），以及 MCP Apps 的沙箱宿主、CSP、`ui/open-link`、`tools/call`（ADR-0052）。
- Bot 的回合状态和 `wonton__report_progress`（ADR-0082/0117）：进度是 Bot 的自述，核心只聚合，不核对。
- 插件通知、来信、文件、配对请求都能写条目，只是现在写进私聊（`gateway/notify.rs:93`、`mailbox/delivery.rs:289-320`、`peers/mod.rs:645-655`、`entry_runtime.rs:200`、`federation_config/mod.rs:124-131`）。

## 调研结论

调研页在 `~/wonton-dev/tasks/inbox-design.html`，九家逐项对比，外加 Claude Code/Slack/Teams/Linear/GitHub 的富条目参照。对决策有用的五条：

1. **富条目是一套协议，不是一条长消息。** Claude 用 `questions[]`、`display`、`recap + actions` 三种结构；Slack 用 block + action_id；Teams 用 Adaptive Card + `Action.Submit`。答案都按 id 回，不靠解析文本。
2. **问题卡要限制形状。** Claude 1–4 题、2–4 项、header 12 字、永远“其他”、可备注、可预览。限制让模型一次写对，也让手机排得下。
3. **网页要当不可信内容。** MCP Apps 的强制沙箱和 CSP：默认 `connect-src 'none'`、`script-src 'self' 'unsafe-inline'`；脚本可跑，联网只去声明的域；不碰 App 的 Cookie 和存储。
4. **通知只放得下“一个决定”。** 无表单确认、单题少选项、收尾按钮可以直接在通知上点；多题、备注、页面必须打开。
5. **答案要结构化、只答一次、可审计。** `{问题: 选中项/自填, 备注}`、`{action_id, comment}`、`acknowledge`；过期、撤回、已答都有明确状态。

我们抄：Claude 的问题形状和 `render/attach`、`wrap_up` 的收尾、Slack 的 action_id 与“答完更新原条目”、Teams 的 `Action.Submit`/正负样式/输入校验、MCP Apps 的 CSP 默认。不抄：Slack 的 50 个 block 和整张 Block Kit、Teams 的完整 Adaptive Card schema、GitHub 的 diff 引擎、Linear 的 Triage 规则。

### 上一版 12 个问题的处置

| 原问题 | 处置 |
| --- | --- |
| 1. 面板范围 A/B/C | 改写：范围固定为收件箱；改成“只放这五种吗” |
| 2. 用户能不能自己加待办 | 删除（不做待办） |
| 3. 能不能交给 Bot 做 | 删除（收件箱没有任务状态） |
| 4. 角标算什么 | 保留，改成“角标只算待回答的条目” |
| 5. 云吞私聊 | 保留：隐藏 + 只读归档 |
| 6. Bot 提问放哪 | 保留：M1 留聊天；`inbox_ask` 是新工具，不强制迁移 |
| 7. 入口放哪 | 保留：顶部一行 + 侧边栏 |
| 8. 和 Hermes Cron/worker 登记表的关系 | 保留并扩成第 5、6 节的桥 |
| 9. 保留多久 | 保留：已处理 24 小时 + 归档 30 天 |
| 10. Snooze | 保留：固定四档 |
| 11. 完成可撤销 | 改写：答案能不能改 |
| 12. know 是否进角标 | 保留：notice 不进角标 |

上一版 5 条建议：私聊退休、入口位置、Bot 提问留聊天、角标、Snooze 全部保留；待办相关的两条（自己加待办、交给 Bot）删除。

新增的富条目问题：谁能发、几张卡/几题/几选项、Other 和备注、页面脚本、页面来源和大小、通知上能答什么、答案能否改、动作条上限。见“要主人拍板”。

## 决策

### 1. 一个条目就是一条有类型、有生命周期、有答案的消息

**条目在核心里**（`core/inbox`，0600），由核心给 `InboxViewModel`，壳只渲染、把操作发回核心。一条条目有：

- `id`（核心分配，带类型前缀 `c-`/`q-`/`p-`/`r-`/`n-`）、`type`、`status`、`created_at`、`expires_at`；
- `sender`（`kind: agent|worker|core|plugin`、`id`、`display_name`、`avatar`）和可选的 `source`（房间、线程、worker_id、任务、深链）；
- `recipient`（一个人或所有管理员）、`read_by`、`answered_by`、`answered_at`；
- `title`、`body`（纯文本兜底，给旧 App、通知和读屏）、`level`（notice）、可选的 `snooze_until`；
- 类型载荷（下表）和 `actions`（0–3 个，带 `id`、`label`、`style`、`requires_comment`）；
- `progress`（可选 0–1，给长任务更新）、`result`（给发送方看的结果引用）。

**五种类型和各自的限制**：

| 类型 | 载荷 | 答案 | 默认期限 | 进角标 |
| --- | --- | --- | --- | --- |
| `choice` | `questions`: 1–4 题；每题 `id` 必填、`header` ≤12 字、`question` ≤200 字、2–4 个 `options`（`id` 必填、`label` ≤40 字、`description` ≤100 字、`recommended?`、可选 `preview` ≤4 KiB），或者 0 个（只有输入框）；`multi_select`（默认 false）、`note_enabled`（默认 false）、`skippable`（默认 false）；永远有 `other`（≤2000 字） | `answers`: 每题 `{selected: [option_id], other?, note?, skipped?}`；整张 `decline` | 阻塞 10 分钟；异步 7 天 | 是 |
| `page` | `resource_uri`（`ui://…`，MCP Apps 资源）或 `file_id`（附件）；`caption` ≤120 字；`network_domains?` 最多 20 个（`https://`、`wss://`，可 `https://*.example.com`）；`actions` 0–3（`id` 限 `[a-z0-9_-]{1,32}`，`label` ≤40 字，`style`: default/primary/destructive，`requires_comment?`） | `{action_id?, comment?}` 或 `acknowledge` | 30 天 | 有 `actions` 时是 |
| `recap` | `summary` 2–3 句、≤400 字；`question?`；`actions` 1–2（同 page） | `{action_id}`、`acknowledge` 或 `decline` | 7 天 | 有问题/动作时是 |
| `confirm` | `title`、`body` ≤2000 字、`detail?` ≤8000 字、`risk`（`high`/`very_high`/`highest`，定义见 ADR-0036）；固定 `choices`（`approve`/`deny`；发送方支持时加 `session`/`always`，最多四个）；`requires_comment?`（为 true 时 `deny` 的答案必须带非空 `comment`，核心校验，壳不能绕过）；`always_ttl?`（`task`（默认）或 `7d`，只在含 `always` 时有效） | 固定 choice；`deny` 的答案带 `comment`（拒绝并说明，≤200 字）；`always` 可带 `always_ttl` | 10 分钟；配对不过期 | 是 |
| `notice` | `title`、`body` ≤2000 字、`level`（`info`/`warning`/`critical`）、可选 `link`（只收 `https:`/`http:`/`wonton:`）、可选 1 个 `action`（同 page） | `acknowledge` 或 `action_id` | 30 天 | 否（只点未读） |

`title` 和 `caption` 都 ≤120 字；所有文本字段超限就报 `invalid_input`，不截断。`choice` 的 `preview` 是**静态** HTML 片段（去掉 `<script>`，≤4 KiB），由宿主放进沙箱渲染；CSP 固定为 `default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'none'`，只允许内联样式和 `data:` 图片/字体，不能跑脚本、不能联网；要脚本或联网就用 `page`。

**生命周期**：

```
pending ──answer──▶ handled ──24h──▶ archived ──30d──▶ 清理
   ├──decline / acknowledge──▶ handled
   ├──expires_at──▶ expired
   └──sender cancel──▶ withdrawn
```

- `handled` 带 `outcome`（answered / declined / acknowledged）和 `answered_by`；管理员条目被别人答了，其余人的那份变 `handled_elsewhere`。
- `expired` 和 `withdrawn` 是终点：再答返回 409，页面显示“已过期 / 对方撤回”。过期后仍留在收件箱 30 天，不能答。
- 一个条目只能答一次。同一个 `answer_id` 重试返回 200 和同一结果；不同 `answer_id` 再答返回 409 并附当前答案，例外只有一种：异步条目第一次 `handled` 后的 60 秒内。
- 重答时旧答案保留为 `superseded`，`GET /inbox/{id}` 同时返回 `answer`（当前）和 `superseded_answers`（旧）；60 秒后旧答案只进审计。
- 阻塞条目在发送方断开 60 秒后自动 `withdrawn`，写明“对方不再等这个回答”；异步条目不因断开撤回。

**谁能看见**：收件人在创建时定下，只有一个人（Agent 的归属人、家人自己）或所有管理员。列表按请求者过滤，家人看不到主人的、别人的，主人也不替家人看。管理员条目对每个管理员都出现，谁先答算谁的。

**核心不做**：项目、看板、标签、assignee、due、重复规则、任务状态、多人分派。要做待办时另开 ADR。

### 2. 谁能发、发多少

| 发送方 | 能发 | 通道 |
| --- | --- | --- |
| 核心 | `confirm`、`notice`、系统 `choice` | 核心内部 |
| 有 `tools` 权限的 Agent/插件 | `choice`、`page`、`recap`、`confirm` | MCP 总线 `wonton__inbox_*` |
| 有 `notify` 权限的插件 | `notice` | `WONTON_NOTIFY_URL`（ADR-0047 不变） |
| router（worker 桥） | `confirm`（worker 等批准）、`recap`/`notice`（完成、卡住） | `POST /_wonton/api/inbox/workers` |
| 用户自己 | 不能直接发；只能答 | App |

**上限**（按发送方算，超了报 `rate_limited`，不截断）：

- 同时挂着的 `choice`/`confirm`/`recap` 合计 5 条，`page` 3 条；超了 `too_many_pending`。
- 新建 10 条/分钟、200 条/天；`notice` 沿用 ADR-0047 的 10 条/分钟、200 条/天。
- 同一发送方带同一个 `dedupe_key` 只留一条：更新内容、重置期限，不新建。长任务用 `inbox_update` 更新同一条，不刷屏。
- 页面大小 ≤1 MiB；附件 ≤20 MiB（走文件服务）。超了 `payload_too_large`。
- `choice` 的 `questions` 和 `options` 数量固定，超了 `invalid_input`；不允许“发 8 个你挑”。

**谁看得见谁发的**：发送方必须是有 Bot 或插件身份的已装应用；核心账号不能伪造。收件人默认是发送 Bot 的归属人，要发所有管理员时带 `audience: admins`（只有核心和声明了 `admin_notice` 的插件能发）。家人自己的 Bot 只能发家人自己。

### 3. Agent 侧接口：MCP 工具

核心在 MCP 总线上加一组自己的工具，名字都是 `wonton__inbox_*`。任何有 `tools` 权限的插件都能调，核心不认识具体的 Agent（ADR-0015）。

```jsonc
// wonton__inbox_ask：问题卡
{
  "title": "照片备份",             // 可选，卡片标题
  "questions": [{
    "id": "q0",
    "header": "目标盘",
    "question": "照片备份到哪块硬盘？",
    "multi_select": false,
    "options": [
      { "id": "nas", "label": "NAS", "description": "容量大，慢一些", "recommended": true,
        "preview": "<div>…</div>" },   // 可选，静态 HTML，≤4 KiB
      { "id": "local", "label": "主机自带的硬盘" }
    ],
    "note_enabled": true,          // 可选备注
    "skippable": true              // 可选跳过
  }],
  "blocking": true,
  "expires_in_ms": 600000,
  "dedupe_key": "photo-backup-1"
}
```

返回（阻塞时等答案，异步时不 wait）：

```jsonc
{
  "item_id": "q-01J…",
  "status": "answered",            // answered | declined | expired | withdrawn | pending
  "outcome": "answered",
  "answers": [
    { "question_id": "q0", "selected": ["nas"], "note": "用 2 号盘" }
  ],
  "answered_by": "…",
  "answered_at": "…"
}
```

其余工具：

```jsonc
// wonton__inbox_present：页面或附件
{
  "title": "备份报告",
  "resource_uri": "ui://photos/report/42",  // 或 file_id
  "caption": "这周的备份报告",
  "network_domains": ["https://cdn.example.com"],  // 可选；不给就断网
  "actions": [
    { "id": "approve", "label": "通过", "style": "primary" },
    { "id": "reject", "label": "退回", "style": "destructive", "requires_comment": true }
  ],
  "blocking": true,
  "expires_in_ms": 2592000000
}

// wonton__inbox_recap：收尾
{
  "summary": "NAS 已挂载，证书还没配。要我继续配证书吗？",
  "question": "继续吗？",
  "actions": [
    { "id": "continue", "label": "继续", "style": "primary" },
    { "id": "stop", "label": "先停" }
  ],
  "blocking": true
}

// wonton__inbox_confirm：固定确认
{
  "title": "安装 Immich？",
  "body": "需要 storage.share.photos 和 network.host。",
  "detail": "…",                    // 原始命令/权限清单，折叠
  "risk": "high",
  "choices": ["approve", "deny", "always"],
  "expires_in_ms": 600000
}

// wonton__inbox_notify：只读通知
{ "title": "备份失败", "body": "…", "level": "warning", "link": "wonton://plugins/immich/backups" }

// 异步取答案、更新、撤回
wonton__inbox_wait    { "item_id": "q-01J…", "after_revision": 12 }
wonton__inbox_update  { "item_id": "q-01J…", "summary": "还在传，2/3", "progress": 0.66 }
wonton__inbox_cancel  { "item_id": "q-01J…", "reason": "任务提前结束" }
```

答案的形状：

```jsonc
{
  "answer_id": "01J…",             // 客户端生成，幂等
  "action": "submit",              // submit | decline | acknowledge | action | approve | deny | session | always
  "answers": [{ "question_id": "q0", "selected": ["nas"], "other": null, "note": "2 号盘" }],
  "action_id": null,
  "comment": null
}
```

- `selected` 是选项 id，不是 label；label 只在界面显示。`other` 是自填；`note` 是备注；单选最多一项，多选 0–4 项。
- 每题默认必答；Agent 可以设 `skippable: true`，跳过就是 `{skipped: true}`。整张 `decline` 不带 `answers`。
- `action_id` 必须在这条条目的 `actions` 里；页面/收尾的按钮就是这样回。`requires_comment` 时答案里的 `comment` 必填（≤200 字，核心校验；`confirm` 上只在 `deny` 时生效）。
- `always_ttl` 只对 `always` 生效：默认 `task`（本任务内有效），`7d` 表示 7 天后失效；核心按发送方给的键（例如设备、来源会话、命令签名）和到期时间记录，换工作区是否重新问由发送方定义。
- `confirm` 的 `action` 直接用档名 `approve`/`deny`/`session`/`always`，不填 `action_id`；`acknowledge` 只对 `notice`、无动作的 `page`、无问题的 `recap` 有效。
- 敏感操作分两种：核心自己的确认卡片（工具放行、装插件、受控发送）由核心在主人允许后继续那份挂起的调用；Agent 用 `inbox_confirm` 只是问一个固定选择，答案回到 Agent，执行由 Agent 侧再做。两种都走同一个 `confirm` 界面和审计，不能互相代替。

**阻塞和异步**：

- `blocking` 的默认值：`inbox_ask`/`inbox_confirm` 为 `true`，`inbox_present`/`inbox_recap` 为 `false`；发送方可以改。`blocking: true` 时工具一直等到 `handled`/`expired`/`withdrawn`，最长 24 小时；核心不主动断 MCP 会话，会话空闲期限要比条目期限长，或由桥定期续。
- `blocking: false` 时立刻返回 `{status: "pending", item_id}`；Agent 之后用 `inbox_wait` 等，或者继续干别的。
- 没有已配对设备时仍创建条目，返回 `pending` 并带 `warning: "no_device"`；不假装送达。主人上线后在收件箱看到。
- 过期、拒绝、撤回都是**正常结果**，不是 `isError`；`isError` 只给发不出去、权限、超限、参数非法。
- 页面资源在创建时由核心经总线 `resources/read` 取回、校验 MIME 和大小，失败就 `isError` 不建条目。条目存的是快照，发送方以后改页面不影响已发的条目。

### 4. HTML 页面：从哪来、放哪、怎么打开

**来源只有两种**：
- `resource_uri`：发送方自己的 MCP 服务器提供的 MCP Apps 资源（`ui://…`，MIME 必须是 `text/html;profile=mcp-app`）。核心按发送方身份 `resources/read`，别的插件的资源不行；不能是公网 URL，M1 不嵌外部网页。
- `file_id`：文件服务里的附件。HTML 文件和页面一样沙箱打开，HTML 文件也 ≤1 MiB；PDF/图片走系统查看器，附件 ≤20 MiB。附件 HTML 没有 MCP 元数据，联网名单要由 `inbox_present` 的 `network_domains` 给，不给就断网。

**联网名单**：页面默认完全断网（`connect-src 'none'`、`frame-src 'none'`、`base-uri 'none'`，无外链资源）。发送方要联网，必须在 `_meta.ui.csp`（`ui://` 资源）或 `network_domains`（文件附件）里列域名；`ui://` 可以再用 `network_domains` 收紧。合并规则：`connectDomains` + `resourceDomains` 合成显示和资源名单（最多 20 个，只收 `https://` 和 `wss://`）；`frameDomains` 只进 `frame-src`，`baseUriDomains` 只进 `base-uri`（两者只收 `https://`）；这两类也进“会访问”显示名单。文件附件 M1 的 `network_domains` 只喂资源和 connect，要 frame/base 用 `ui://` 资源。不写路径；通配只允许 `https://*.example.com` 这种子域。核心把最终名单存进条目并生成 CSP：`https` 的资源域进 `script-src`/`img-src`/`style-src`/`font-src`/`media-src`，`https`/`wss` 的 `connectDomains` 进 `connect-src`。名单为空就是完全断网，只允许内联脚本、`data:` 图片和 `data:` 字体。页面运行时不能再加域。

**存哪**：核心在发送时把 HTML 取回来，存进 `core/inbox/pages/<item-id>.html`（0600，最大 1 MiB，随条目 30 天后删）。不依赖发送方以后还在不在；发送方删了插件，页面还能打开，但页面里的 `tools/call` 会失败并显示“插件连不上”。页面不是 ADR-0052 的“插件页/面板页”，是条目自己的快照，故不进面板、不共享存储。

**怎么打开**：App 当 MCP Apps 宿主，复用 ADR-0052 的沙箱宿主：

- 单独的、不留持久存储的 WebView；页面放进 `sandbox` 的 iframe；按 MCP Apps 规范给 `allow-scripts` 和 `allow-same-origin`，不给 `allow-top-navigation`、`allow-popups`。
- 核心按名单生成 CSP：名单为空时用 MCP Apps 的默认
  `default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; media-src 'self' data:; connect-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'`；
  有名单时把资源域追加到 `script-src`/`img-src`/`style-src`/`font-src`/`media-src`，`connectDomains` 追加到 `connect-src`，`frameDomains` 覆盖 `frame-src`，`baseUriDomains` 覆盖 `base-uri`。
- 页面可以跑内联脚本（图表、折叠、表单校验）；页面要自带 CSS/JS，图片用 `data:` 或名单里的绝对地址，M1 不解析 `ui://` 子资源。外链脚本/图片/字体/CSS 和 `fetch`/WebSocket 都只在名单里；没声明的域直接被 CSP 拦。`permissions` 只透传 `clipboardWrite`；camera/mic/geolocation M1 不给。
- 列表行和详情顶部都显示“会访问：a.com、b.com”；名单为空写“不联网”。锁屏通知写“会访问 N 个站点”或“不联网”。用户点“打开”前就能看见。
- `ui://` 的名单只来自资源 `_meta.ui.csp`；`network_domains` 对 `ui://` 是可选的收紧名单（交集），多写的域被忽略，少写的域会被去掉。文件附件没有元数据，名单只认 `network_domains`。核心把最终名单校验、存快照。
- 页面不能读 App 的 Cookie、钥匙串、文件；没有 `localStorage` 的跨条目持久化（WebView 每次打开都是干净 profile；`sessionStorage` 只在这一次打开有效）。
- 外链一律 `ui/open-link` 交给系统浏览器；`target=_blank` 由宿主接住。页面自己不能做顶层跳转。
- 页面可以经宿主调 `tools/call`，只限同一个 MCP 服务器上 `_meta.ui.visibility` 含 `app` 的工具，经核心总线转发；调用方记成发送方 Bot（ADR-0052 的例外，见 0052 状态行），敏感工具照常走确认。页面调用工具产生的确认是新的一条 `confirm`，嵌套最多一层。页面里的 `<a>` 由宿主接住、走 `ui/open-link`；`<form>` 不在页面里提交，只做本地校验。
- 页面可以 `ui/message` 以主人名义在来源会话发一条消息，但**答案不走 `ui/message`**。

**答案**：页面底下是 App 原生的动作条，不是页面自己画的提交按钮。动作来自条目的 `actions`，由核心校验 `action_id`；页面也可以 `postMessage` 一个 `ui/notifications/inbox-action` `{item_id, action_id, comment?}` 提前选好 `action_id`，但最终由原生条提交，核心按 `action_id` 再校验一次。答案只回核心，页面拿不到也能被审计。

**和插件网页、ADR-0038 的关系**：页面复用 ADR-0052 的 MCP Apps 宿主和 CSP，但比插件网页更窄：无持久存储、无公网 URL、无顶层跳转、只活 30 天。问题卡直接复用 ADR-0038 的 `StepCard` 形状和 `AskUserQuestion` 上限，只是从“Bot 聊天里的面板”改成“收件箱里的 `choice` 条目”；原来的 `wonton__ask_owner` 改名为 `wonton__inbox_ask`，上限和选项形状沿用（字段名从 `steps` 改为 `questions`）；M1 没有外部用户，不做别名。

### 5. 和 Hermes 的关系：通用工具 + 薄桥

核心不为 Hermes 特殊处理（ADR-0015）。分两条路：

**5.1 Wonton 自己的问题卡**：关掉 Hermes 自带的 `clarify` 工具集，模型改用核心的 `wonton__inbox_ask`。答案作为工具结果回到 Hermes 的模型，Hermes 继续干；Hermes 不需要知道收件箱存在。

**5.2 Hermes 原生的批准/澄清**：Hermes 的网关发 `approval.request` / `clarify.request` 事件，用 `approval.respond` / `clarify.respond` 回答。`approval.pending` 是 Desktop 的补播请求（不是事件）：入参 `session_id`+`profile`，返回 `{approvals:[{request_id,…}]}`，桥在 attach 时用它补没收到的事件。一个薄的 Hermes 插件做桥（以我们镜像钉的 v0.22.7 源码为准：`gateway/session_pending_controls.py:38`/`:51` 发请求，`gateway/session_controls.py:842-852` 收回答，`acp_adapter/gateway_server.py` 的 `_answer_clarify`/`_answer_permission` 是调用样板）：

- 订阅 `approval.request` / `clarify.request` 事件，把 `prompt_id`、`execution_generation`、`session_id`、命令或问题、`choices`/`multi_select` 写成 `confirm`/`choice` 条目（`clarify` 的 `choices` 为空就是 0 选项输入框；Hermes 的题一律设 `skippable: true`），`source` 带 `session_id`/`prompt_id`/`execution_generation`；条目的 `sender` 是 Hermes，`dedupe_key` 用 `prompt_id`。`approval.request` 的 `choices` 只映射它实际给的档：`once`→`approve`、`session`→`session`、`always`→`always`、`deny`→`deny`；没给的档不显示。
- **`clarify.request` 是一题一个事件，不拆卡。** Hermes 的 `clarify` 工具一次可以带 5 题，但网关每题发一个 `clarify.request`；桥就每题建一条 `choice`（1 题），`dedupe_key` 用 `prompt_id`。4 题上限只约束 `inbox_ask`，不约束 Hermes 的原生事件。主人在收件箱按顺序答，每题答完立刻调一次 `clarify.respond`；Hermes 自己数到最后一题继续，桥不合并、不等全部。
- 用户点跳过时，`answer` 发 `"\x00skipped"`（`tools/clarify_gateway.py:45` 的 `SKIPPED`；runner 在 `gateway/run_turn_runner.py:592` 存成 None 并继续下一题）；`decline` 是 `"\x00cancelled"`（同文件 `:46` 的 `CANCELLED`，runner 在 `run_turn_runner.py:580` 认）；`expired` 是空串 `""`（`gateway/run_turn_runner_clarify_delivery.py:115` 把它当没答、走 `timed_out`）；正常答案是选中项原文，默认 `submitted`。`clarify.respond` 没有 `outcome` 字段，outcome 是 Hermes runner 按这三种值推的，别在 RPC 里多加字段。
- `clarify` 是阻塞条目，60 秒内不可重答（和全局一致）；要改只能等 Hermes 再问或另发一题。
- 主人在收件箱答完，桥调 `approval.respond`（`{session_id, execution_generation, prompt_id, choice}`，`choice` 就是上面映射的档）或 `clarify.respond`（`{session_id, execution_generation, prompt_id, answer}`；`answer` 是字符串：选中项原文，`Other` 是自填文本，多选用 JSON 数组字符串）。Hermes 的网关按现有逻辑继续那一轮。`on_session_prompt` 这类 UI 层钩子只影响本地展示，桥以网关的 `*.request`/`*.settled` 事件为准。
- 另一端先答了（本地 CLI/桌面）：网关广播 `approval.settled` / `clarify.settled`（`gateway/session_pending_controls.py:63`；ACP viewer 的 `_project` 也收这两种）；桥收到 `{prompt_id, execution_generation}` 后 `inbox_update` 成 `handled_elsewhere`，收件箱里撤下按钮。`clarify.lock` 是另一条 RPC（抢同一请求的回答权），不是广播事件。
- Hermes 进程退出、轮次中断：桥在 60 秒后 `inbox_cancel`，条目写“Hermes 不再等这个回答”，不假装还阻塞。
- 桥只做映射，不做权限判断：`always` 的持久化、`session` 的范围、危险命令的判断全在 Hermes；核心只把 choice 原样送回。

`approval` 不拆卡：它是单个 `confirm`，答完直接 `approval.respond`，没有部分完成的问题。

**5.3 异步**：Hermes 的 `clarify` 本身是阻塞工具，桥默认也用阻塞；`inbox_ask(blocking:false)` 只给愿意自己 `inbox_wait` 的 Agent。M1 不要求 Hermes 支持异步。

`approval.respond` 和 `clarify.respond` 的参数名以 Hermes 源码为准（`session_id` + `execution_generation` + `prompt_id` + `choice`/`answer`），桥升级时跟着 Hermes 的网关协议走；核心不解析 Hermes 的字段。

### 6. worker/router 怎么推

worker 没有 MCP，用 HTTP，主人令牌或经隧道的设备身份：

```http
POST /_wonton/api/inbox/workers
{
  "worker_id": "w-01J…",
  "event": "needs",                  // needs | done | stuck
  "name": "herdr",
  "task": "整理 2024 照片",
  "place": "pane 3 / worktree photos",
  "title": "worker 等你批准",
  "body": "要删除 12 张重复照片",
  "risk": "high",
  "choices": [
    { "id": "approve", "label": "允许", "style": "primary" },
    { "id": "always", "label": "总是允许" },
    { "id": "deny", "label": "拒绝", "style": "destructive" }
  ],
  "expires_at_ms": 1760000000000
}
```

- `needs` → `confirm`；`done` → `notice` 或 `recap`（带结果链接，M1 不从 router 发 `page`）；`stuck` → `confirm`（重试/停止/打开）。worker 的 `confirm` 必须带 `risk`，没给按 `high`。
- 同一个 `worker_id` 的同一类事件只留最新一条；连续 `stuck` 更新那一条，不每次刷一条。
- 答案怎么回：条目详情给 runner 自己的按钮语义（`approve`→`tern approve allow`、`always`→`tern approve always`、`deny`→`tern approve deny`、herdr 的“回车/Esc”）。router 轮询 `GET /_wonton/api/inbox/workers/{worker_id}` 拿 `{status, answer}`；执行成功 `POST …/workers/{worker_id}/resolve {delivery:"sent"}`，执行失败 `{delivery:"failed", error}`，条目保持 pending 并显示“没送到，重试”。
- router 不把登记表塞进核心：核心只存 `worker_id`、name、task、place、最后事件，App 不查 router；要交互就“打开终端”，走 router 的深链。
- 桥死、宿主重启、登记表对不上：对账补 `WorkerStuck`，不自动把 worker 当完成。

### 7. 消息、来信、配对、家人：现有来源怎么映射

| 来源 | 类型 | 说明 |
| --- | --- | --- |
| 插件工具、装/卸/升级、存文件、受控发送、链上限 | `confirm` | 保持 ADR-0016/0024 的确认和审计；卡片在来源会话仍有一行投影，收件箱一份事实 |
| Bot 在聊天里的提问 | `choice`（M1 仍可留聊天） | 用 `inbox_ask` 就进收件箱；不用就在聊天里问，不强制迁移 |
| 安装、升级、发送、存文件的结果 | `recap` / `page` / `notice` | 有下一步用 `recap`，有报告用 `page`，只是告知用 `notice` |
| 插件通知（下载完成、备份失败、证书到期） | `notice` | 级别沿用 ADR-0047；现有的 `WONTON_NOTIFY_URL` 调用仍返回 200，只是落成条目、不再写私聊，插件不用改 |
| 应用来信（找回密码、验证码） | `notice` | 正文里给验证码，链接用 `ui/open-link` |
| 文件发到另一台设备 | `page`（attach） | 附件卡，不再发私聊 |
| 入口对账失败、联邦名单失败、磁盘将满、模型 key 失效 | `notice` | warning/critical，不占角标 |
| 好友、家人加入/移除、Bot 联系请求 | `confirm` | 同意/拒绝/只聊天；管理员条目谁先答算谁的 |
| 别的服务器配对请求 | `confirm` | 不过期；重置码是拒绝后的动作 |
| 新设备登录、密钥过期 | `confirm` | critical，允许/拒绝/吊销 |
| 插件更新可用、核心升级 | `notice` | 打开插件页/release notes |
| Bot 建议的任务 | `choice` | “要不要我每周备份？”接受/忽略；没有任务状态 |
| worker 完成、卡住、等批准 | `notice` / `recap` / `confirm` | 见第 6 节 |
| 壳不认识的类型 | `Other` | 不显示但计数，旧壳只显示 `body` 兜底 |

`notice` 的级别、频率上限、去重沿用 ADR-0047；`confirm` 的来源核对沿用 ADR-0016；`choice` 不授权，只问。

`choice` / `page` / `recap` 只进收件箱，不在聊天里再放一行，避免两份事实；`confirm` 保留来源会话的卡片投影，两边同一份回写。

### 8. 通知、锁屏和列表

**三处各显示什么**：

| 位置 | `choice` | `page` | `recap` | `confirm` | `notice` |
| --- | --- | --- | --- | --- | --- |
| 锁屏通知 | 发送方 + 第一题 + “共 N 题”；无表单≤3 选项时给按钮，否则“打开作答” | 发送方 + 标题 + “报告”；“打开” | 摘要前 2 句 + 问题；≤2 个按钮 | 标题 + 风险行；允许/拒绝（+总是/本会话） | 标题 + 摘要；有 action 给 action，另一个位置给“知道了”；有 link 且有空位给“打开” |
| 收件箱列表 | 发送方、标题、第一题、“N 题”、“对方在等”/“只是告知”、到期倒计时 | 发送方、标题、说明、页图标 | 发送方、摘要、问题 | 发送方、标题、风险、“对方在等” | 发送方、标题、级别、未读点 |
| 详情 | 完整题目、选项（推荐置顶）、其他、备注、提交/不回答 | 沙箱页面、动作条、来源 | 全文、问题、按钮 | 全文、风险、详情折叠、允许/拒绝 | 全文、链接、知道了 |

**通知正文**按 ADR-0118 去换行、截 120 字；页面不塞 HTML，只给标题和说明；锁屏不露页面内容。通知类别：`confirm`/`choice`/`recap` 用确认类别，`blocking: true` 用 Time Sensitive；`notice` 用插件通知类别按级别；`info` 不推。

**通知上的动作**：

- `confirm`：允许/拒绝（发送方给了 `session`/`always` 才显示“本会话”/“总是”），直接答；失败留“没答上，点开重试”，条目仍 pending。
- `choice`：只有单题、≤3 个选项、无备注要求时给选项按钮；多题/有备注只给“打开作答”。通知上点选项等于选那一项并提交，`other` 仍要打开。
- `recap`：≤2 个 action 直接给按钮；否则打开。
- `page`：只给“打开”；页面底下的动作在详情里。无动作页面给“知道了”。
- `notice`：有 `action` 时给这个动作，另一个位置留给“知道了”；有 link 且仍有空位时再给“打开”。没有 action 时只给“知道了”（有 link 再加“打开”）。
- 系统按钮最多 3 个（Apple 上限 4，留一个给“打开”）。核心保留的 `approve`/`deny`/`session`/`always`/`acknowledge`/`open` 由 App 本地化；插件自己的 `action_id` 用发送方给的 `label`（≤40 字），未知 id 用 label 兜底。
- 答完立刻把通知从其他设备撤下（`revision` 长轮询）；答的动作也写审计。

**列表**：按“阻塞且最快到期 → 阻塞 → 非阻塞问题 → 有动作的页面 → 通知 → 已处理”排；同组内按到期时间/时间倒序。阻塞条目显示“对方在等”和倒计时；非阻塞显示“只是告知”。第一版不做搜索、筛选、分组设置、全部已读。

**手势**：右滑是主要动作（通知已读、页面已读、问题“稍后”），左滑是“更多”（不回答/删除/打开来源）；长按多选，批量“已读/归档”。批准/拒绝不藏进滑动。

`snooze` 只对 `pending` 条目，只有四档：`hour`（+60 分钟）、`later_today`（本机时区今天 18:00；18:00 后按 +3 小时）、`tomorrow`（本机时区明天 09:00）、`next_week`（本机时区下周一 09:00）。App 传 `{preset, timezone_offset_min}`，核心算 `snooze_until`；没有偏移就用服务器时区。它只把条目挪出列表，**不延长 `expires_at`**；阻塞条目回来时可能已经过期，界面上写剩余时间。

**空态**：托盘勾图标 + “都处理完了”，不加副标题；已处理为空时那一段不显示。读屏读出类型、发送方、是否阻塞、到期和按钮。

### 9. 入口和角标

- **入口**：标题就叫“收件箱”，聊天列表顶部固定一行，图标用 Phosphor 的托盘；侧边栏同一行在“新聊天”下面。不放回底栏（ADR-0095）。
- **角标 = 待回答的条目**：`choice`、`confirm`、有问题的 `recap`、有动作的 `page`。`notice` 和无动作的 `page` 不进角标，只在列表里点未读。应用角标（ADR-0101 第 3 节）是“未读消息 + 这个数”，两者都清零才归零。
- 打开收件箱不归零；逐条答、逐条读才减。全部处理完入口变灰、角标清零。
- `blocking` 条目的角标和通知更醒目，但不额外计数；`handled_elsewhere` 立即从别人的角标里减。

### 10. “云吞”私聊怎么处置

- 停止写入，从聊天列表隐藏，房间保留为只读归档；收件箱最下面“旧的通知记录”打开。旧信里的找回密码链接、文件链接继续能点。
- 家人各自的私聊同样处理；家人的旧记录只有家人自己看得到。
- 第一版不删。90 天后另开裁剪任务：确认没有活跃链接、导出审计后再删。
- 旧 App/旧核心：新 App 对旧核心收件箱 404，照实显示加载失败；旧 App 见到新 `type` 把 `body` 当普通消息显示，附“在最新版 App 回答”。

### 11. 保留、归档、审计

- 待回答的 `choice`/`confirm`：阻塞默认 10 分钟、异步 7 天，最长 24 小时；`page`/`recap` 7–30 天；`notice` 30 天。
- `handled` 后留 24 小时可回看/撤销“已读”，之后进 `archived` 30 天；`expired`/`withdrawn` 留 30 天。
- 页面快照随条目 30 天后删；附件按文件服务的保留策略。
- 审计写：条目 id、类型、发送方、收件人、动作、`answer_id`、结果、投递状态（送到/撤不回/失败）、页面内容的 sha256，不写页面内容和自由文本（避免验证码进日志）；审计留 90 天。
- 答案默认是最终的：阻塞条目的工具结果已经回去，不能再改；只有异步条目第一次 `handled` 后 60 秒内可以撤回重答。重答用新 `answer_id`，旧答案标 `superseded` 并留在 `superseded_answers`，`answer` 换成新答案；60 秒后再答返回 409 `answer_locked`。

### 12. 契约、提供方、消费方

- **契约**：`wonton-protocol` 的 `inbox` 模块（`InboxItem`、`InboxType`、`ChoicePayload`、`PagePayload`、`RecapPayload`、`ConfirmPayload`、`NoticePayload`、`InboxList`、`InboxViewModel`、路径、答案结构）。契约测试：未知类型落 `Other`、新增字段有默认值、管理员条目 `handled_elsewhere`、按收件人过滤、`notice` 已读按人分开、`choice` 过期不能答、`answer_id` 幂等、答案字段不合法 400、页面超过 1 MiB 拒绝。
- **接口**：
  - `GET /_wonton/api/inbox?after=<revision>`，长轮询 25 秒，返回 `InboxList { items, counts:{needing, unread_notices}, revision }`；
  - `GET /_wonton/api/inbox/{id}` 取一条；
  - `GET /_wonton/api/inbox/{id}/page?token=<短令牌>` 取页面（CSP 头、`text/html`）；token 5 分钟、绑条目，重载时 App 换新 token；
  - `POST /_wonton/api/inbox/{id}/answer`、`/acknowledge`、`/read`、`/cancel`；`POST /_wonton/api/inbox/{id}/snooze` 体是 `{preset: "hour"|"later_today"|"tomorrow"|"next_week", timezone_offset_min?}`；
  - `POST /_wonton/api/inbox` 核心内部建条目；`POST /_wonton/api/inbox/workers` 给 router，`GET /workers/{worker_id}` 取答案、`POST /workers/{worker_id}/resolve` 回投递结果；
  - `confirm` 旧的回答接口 `POST /_wonton/confirmations/{id}` 继续可用，条目状态由回写驱动，壳不直接改条目。
- **提供方**：核心。现有确认表、通知入口、来信投递、配对请求、联系人请求、升级流程都改成写条目，不再写私聊。
- **消费方**：两端壳（收件箱页、通知动作、页面宿主）和客户端核心（长轮询、ViewModel、页面加载）。
- **换提供方不改消费方**：类型和 `action_id` 是核心给的，壳按类型渲染；以后加 `form` 类型或新动作不用改壳骨架。
- **鉴权**：用户操作只认经隧道的已配对设备或主人令牌；Agent 工具认 Bot 身份；worker 认主人令牌或 router 的隧道身份；页面 token 只读、5 分钟、绑定条目。
- **推送**：一个条目一条通知；更新只替换不重弹；`notice` 的 `info` 不推；`critical` 用 Time Sensitive；正文只带标题和摘要，不带 HTML。
- **隐私**：条目在核心里是明文，核心用设备公钥加密后交给中继（ADR-0118）；锁屏内容分类和 App 锁（ADR-0101 第 5 节、ADR-0102）照旧；页面在锁定期间不露内容。

### 13. 第一版不做

表单类型、公网 URL 页面、页面持久存储、页面自动提交、多人分派、项目/任务状态、搜索、筛选、分组设置、全部已读、富文本编辑器、附件内联预览（HTML 以外走系统查看器）、嵌套超过一层、Agent 自动放宽确认。

## 拍板结果（2026-11-25）

2026-11-25 主人拍板：下面 20 题全部按推荐；第 6 题 23:4x 补定为“脚本可跑、联网只按名单、卡上写明、无名单断网”。

1. **收件箱只放这五种吗**：五种（`choice`/`page`/`recap`/`confirm`/`notice`，推荐）；再加 `form` 自由表单；再加 `task` 待办。**推荐五种**，先把结构跑通。
2. **谁能发**：任何有 `tools` 权限的 Agent（推荐，配上限）；只有核心和声明了 `inbox` 的插件；每次主人点“允许这个 Bot 发”。**推荐第一种**。
3. **一张卡最多几题**：1–4 题（Claude 的上限，推荐）；1–5 题（照顾 Hermes `clarify`）；1 题。**推荐 4 题**，多出来的让 Agent 拆两张；Hermes 原生事件一题一条，不受此限。
4. **每题最多几个选项**：2–4 个，允许 0 个自由回答（推荐）；2–6 个；不限制。**推荐 4 个**，多了让 Agent 合并。
5. **“其他”和备注**：每题永远有“其他”，备注可选（推荐）；“其他”永远有，备注必填；只在没有选项时给“其他”。**推荐第一种**。
6. **页面能不能跑脚本**：脚本可以跑；联网只按发送方声明的域名名单放行，卡上写明“会访问：xxx”，不写名单就完全断网。**已按此定。**
7. **页面从哪来**：只收 MCP Apps 的 `ui://` 资源 + 文件附件（推荐）；也收公网 URL；也收上传的 HTML 文件。**推荐第一种**，公网 URL 后加。
8. **通知上能答什么**：无表单确认 + 单题≤3 选项 + 收尾按钮（推荐）；只有确认卡片；只给“打开”。**推荐第一种**。
9. **答案能不能改**：提交即最终；异步条目 60 秒内可撤回（推荐）；发送方未读前都能改。**推荐第二种**（只给异步）。
10. **入口叫什么**：收件箱（推荐，主人的词）；待处理；通知。**推荐收件箱**。
11. **角标算什么**：待回答的条目（推荐）；全部未处理含通知；全部未读。**推荐第一种**；通知不进角标。
12. **保留多久**：已处理 24 小时 + 归档 30 天（推荐）；7 天 + 90 天；页面永久。**推荐第一种**。
13. **“云吞”私聊**：隐藏 + 只读归档（推荐）；保留为普通聊天；30 天后删除。**推荐隐藏 + 归档**，先不删。
14. **Bot 在聊天里的提问**：留聊天（M1，推荐）；镜像到收件箱但回答仍在聊天；全部搬进收件箱。**推荐留聊天**，`inbox_ask` 是新工具，不强制迁移。
15. **Snooze**：固定 1 小时 / 今天晚些 / 明天 / 下周（推荐）；自由日期；不做。**推荐固定四档**。
16. **页面最大多大**：256 KiB；1 MiB（推荐）；5 MiB。**推荐 1 MiB**。
17. **页面动作条最多几个**：0–3 个（推荐）；固定一个；不限制。**推荐 3 个**。
18. **阻塞默认**：发送方选，`ask`/`confirm` 默认阻塞（推荐）；全部阻塞；全部异步。**推荐第一种**。
19. **收尾框最长**：2–3 句（推荐）；5 句；不限。**推荐 3 句**。
20. **通知能不能带一个动作**：可以（推荐）；只能“知道了”；不能动作。**推荐可以**，动作仍走 `action_id`。

## 没选的做法

- **只做纯文本通知。** 主人已经否了：问题、选项、报告、下一步都要结构化。
- **把条目做成任务（assignee/due/status）。** 主人这次明确“就是收件箱”；任务层以后另开 ADR。
- **页面用公网 URL。** 要处理登录、CSP、跨域和离线；M1 先只收 MCP Apps 资源和文件。
- **页面存在发送方，不存核心。** 发送方删插件、重启、改页面后旧条目会烂；存快照 30 天，代价是核心多存最多 1 MiB。
- **页面直接 POST 答案。** 页面没有隧道身份，答案只能经宿主；否则要给它发令牌、做重放保护。
- **通知上直接做多题或填备注。** 系统通知装不下，而且 `Other` 需要键盘。
- **一套自由表单 schema。** Teams Adaptive Card 已经证明可行，但第一版用 1–4 题选择题 + 备注就够；自由表单后加。
- **让页面自己画提交按钮。** 每个插件各画一套，高度、禁用态、错误态都不一致；原生动作条统一。
- **用 `ui/message` 当答案。** 消息会进聊天时间线，核心还得解析；答案走结构化接口。
- **保留“云吞”私聊继续塞。** 私聊清了找不回来，长信挤聊天流，通知和收件箱两份事实。
- **角标算所有通知。** 通知一多角标永远清不掉，用户会忽略；只算待回答的。
- **一件事发好几条（进度一条、结果一条）。** 用 `dedupe_key` + `inbox_update` 更新同一条。
- **让 Agent 设必答/必填。** 主人可能整张不回答；每题可跳过，`Other` 永远在。

## 代价与风险

- **页面是新的攻击面。** 靠沙箱、CSP、无存储、短令牌、原生动作条兜；页面仍可能诱导点击，答案必须经核心校验 `action_id`。
- **核心开始存内容。** 页面快照 1 MiB × 30 天，最坏几十 MB；用 `core/inbox/pages` 配额和清理兜底。
- **富条目让模型更容易话多。** 一张卡 4 题、4 选项、页面、备注，可能比聊天更长；靠工具说明“一件事一张卡，简单事聊天里问”。
- **Hermes 桥是新的单点。** 网关 RPC 变了要跟；桥死时条目会过期，但不会假装答了。桥做成能被 supervisor 拉起的长驻插件。
- **页面动作和工具调用两套。** 页面里的按钮可能调工具（副作用），底下的动作只是答案；要在 UI 上分开写，避免用户以为“通过”已经执行。
- **通知上误点。** 选项按钮直接提交；给 3 秒“已答，撤销”？M1 用 60 秒异步撤回，阻塞不可；误点要重新问。
- **旧 App 答不了。** 旧 App 只看到 `body` 兜底和“在最新版 App 回答”；发卡片的 Bot 要接受这个降级。
- **页面快照会过时。** 发送方后来改了页面，旧条目不变；这是“消息不变”的代价，条目里写明生成时间。
- **家人看不到主人的、主人也看不到家人的。** 管理员条目靠 `handled_elsewhere` 收敛；家人多的家庭会重复推送，这是 ADR-0088 的代价。
- **`notice` 仍可能刷屏。** 频率上限沿用 ADR-0047，`info` 不推、不占角标；批量已读。
- **没有搜索。** 30 天后归档，找旧条目靠来源会话和类型；量上来再加。

## 待验证

- 390×844 下 4 题卡、4 选项、长 header、备注键盘、页面动作条；大字号不截断。
- 通知上的允许/拒绝/单题选项能答；答完另一台设备撤下；答失败留“重试”。
- 页面沙箱：内联脚本能跑；未声明的 `fetch` 被 CSP 拦；`ui/open-link` 走系统浏览器；页面调 `tools/call` 记成发送方 Bot；敏感工具另弹确认。
- 页面快照 30 天清理；发送方删插件后页面还能打开、工具调用显示“插件连不上”。
- Hermes 桥：`approval.request` 变 `confirm`、`clarify.request` 逐题变 `choice`；答完分别调 `approval.respond`/`clarify.respond`；本地 CLI 先答时收到 `*.settled` 变 `handled_elsewhere`；Hermes 退出 60 秒后 `withdrawn`。
- worker `needs` 变 `confirm`；答完 router 轮询拿到并 `tern approve`；执行失败留 pending 和“没送到”。
- 管理员条目 A 答后 B 显示“已由 A 处理”且通知撤下；家人 A 的条目不出现在家人 B 的收件箱。
- 无设备时 `inbox_ask` 返回 `pending` + `no_device`，不假装送达；上线后能看到。
- `answer_id` 重试幂等；不同 `answer_id` 409；过期/撤回后答 409。
- 未知类型落 `Other`、旧壳不白屏；新增字段有默认值；页面 >1 MiB 拒绝。
- 端到端：装插件、收一封应用来信、触发一条 `critical` 通知、另一台服务器发配对请求，四件事都只进收件箱、私聊为空；再发一张 4 题卡和一个带动作的 HTML 页面，答完都留结构化答案和审计。

## 查证来源

- 调研页（九家逐项对比、富条目参照、完整来源）：`~/wonton-dev/tasks/inbox-design.html`。
- Claude Code `AskUserQuestion`：https://code.claude.com/docs/en/agent-sdk/user-input （1–4 题、2–4 选项、header 12 字、`Other`、备注、`preview`、答案按题目回）。
- Claude Code `SendUserFile`：https://code.claude.com/docs/en/tools-reference （`display: render|attach`、caption）；工具说明：https://github.com/Piebald-AI/claude-code-system-prompts/blob/main/system-prompts/tool-description-senduserfile.md。
- Claude Code `wrap_up`：https://github.com/Piebald-AI/claude-code-system-prompts/blob/main/system-prompts/tool-description-wrapup.md （2–3 句 recap、一个问题、按钮）；桌面端 next-step 框的问题：https://github.com/anthropics/claude-code/issues/103169。
- MCP Apps 规范（CSP 默认、沙箱、`ui/open-link`、`tools/call`）：https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx 。
- 现有 MCP Apps 宿主、页面安全：`docs/adr/0052-chat-pull-panel.md`（“Bot 往面板上放页面”“MCP Apps 界面怎么显示”）。
- 问题卡形状和上限：`docs/adr/0038-multi-step-cards.md`（`wonton__ask_owner`、`StepCard`、超时不提交、Other 永远有）。
- Hermes 本地源码（commit `b200f5bc`）：`tools/clarify_tool.py:6-7`（`MAX_CHOICES = 4`、`MAX_QUESTIONS = 5`）、`tools/clarify_gateway.py:45-46`（`SKIPPED = "\x00skipped"`、`CANCELLED = "\x00cancelled"`）、`tools/approval_prompt.py:37`（`once`/`session`/`always`/`deny`/`timeout`/`cancelled`）、`gateway/session_pending_controls.py:38/51/63`（`*.request`/`*.settled`、一题一个 `clarify.request`）、`gateway/session_controls.py:842-852`（两个 respond 的必填参数）、`gateway/run_turn_runner.py:580/592`、`gateway/run_turn_runner_clarify_delivery.py:115`（cancelled/timed_out 的约定值）、`acp_adapter/gateway_server.py` 的 `_project`/`_answer_clarify`/`_answer_permission`、`gateway/session_commands.py:516`（另一端先答后 settle 所有 viewer）。
- Slack Block Kit：https://docs.slack.dev/reference/block-kit/blocks/ （最多 50 block、元素类型）；Actions block：https://docs.slack.dev/reference/block-kit/blocks/actions-block （最多 25 元素、`action_id`）；交互处理：https://docs.slack.dev/interactivity/handling-user-interaction/ （`payload`、3 秒 ack、`response_url` 30 分钟 5 次、`replace_original`）。
- Teams Adaptive Cards：`Input.Text` https://learn.microsoft.com/en-us/adaptive-cards/schema-explorer/input-text （`id`、`isRequired`、`errorMessage`、`maxLength`）；`Action.Submit` https://learn.microsoft.com/en-us/adaptive-cards/schema-explorer/action-submit （收集输入、`style`、`mode`、`isEnabled`）；卡片版本/移动端上限：https://learn.microsoft.com/en-us/microsoftteams/platform/task-modules-and-cards/cards/cards-reference （移动端最高 1.6）。
- Linear Inbox：https://linear.app/docs/inbox （读/未读、Snooze、删除、Priority/Other）。Linear Triage：https://linear.app/docs/triage （Accept/Decline/Duplicate/Snooze）。
- GitHub PR review：https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/reviewing-proposed-changes-in-a-pull-request （逐文件 review、Viewed、Approve/Request changes）；建议更改：https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/incorporating-feedback-in-your-pull-request （建议、批量提交）。
- Apple HIG `Notifications`：https://developer.apple.com/design/human-interface-guidelines/notifications （最多四个动作、角标只表示未读、不重复通知）。

## 关联

ADR-0003（推送）、ADR-0005（厚核心）、ADR-0015（小组件）、ADR-0016（确认卡片）、ADR-0038（StepCard）、ADR-0045（家人）、ADR-0047（插件通知）、ADR-0050（配对请求）、ADR-0052（MCP Apps 宿主）、ADR-0056（内部邮箱）、ADR-0082（运行状态）、ADR-0088（管理员）、ADR-0095（没有底栏）、ADR-0101（通知动作/角标）、ADR-0102（App 锁）、ADR-0117（Bot 长任务）、ADR-0118（推送带正文、房间不加密）、ADR-0026/0029（外观）。
