## 话题会话（ADR-0058）发进聊天和拼给 Agent 的文字。

name-main = 主对话

topic-no-active-session = 没有活动 ACP session
topic-input-too-long = 输入超过 { $size } 字节限制
topic-input-empty = 输入不能为空
topic-control-no-text = 控制命令缺少文字
topic-session-lost = ACP 会话已丢失，已新建会话；之前的上下文不再可用。
topic-queued = 话题已排队：现在有 { $ahead } 个活跃话题，有话题结束后自动开始。
topic-permission-failed = Agent 权限请求失败：{ $message }
topic-permissions-cancelled = 权限请求已取消，Agent 没有执行该操作。
topic-output-limited = 上游错误：Agent 回复超过输出限制，已取消本轮。
topic-no-text = Agent 本轮没有产出文本（stopReason={ $reason }）。
topic-idle-ended = 话题空闲超过 30 分钟，已结束。
topic-main-cannot-end = 主对话不能结束。
topic-already-ended = 这个话题已经结束了。
topic-ended-running = 话题已结束；这一轮还在跑，结果还发进这个话题。
topic-ended = 话题已结束。
topic-ended-idle = 空闲超过 30 分钟
topic-ended-manual = 用户手动结束

status-working = 进行中
status-waiting = 等你确认
status-idle = 空闲
status-finished = 已结束
status-queued = 排队中

conflict-reject-reason = 「{ $other }」正在改 { $path }，用户选择不改
conflict-reject-reason-short = 用户选择不改
conflict-approved = 「{ $name }」这一轮做完了，已批准改 { $path }。
conflict-in-use-title = { $title }（「{ $other }」也在改）
conflict-wait-option = 等「{ $other }」做完再改
conflict-now-option = 现在就改
conflict-reject-option = 不改
conflict-in-use-note = 「{ $other }」正在改 { $path }；它这一轮做完前改这里可能被覆盖。

digest-header = 其他会话的动态：
digest-working = 「{ $name }」正在做：{ $doing }
digest-waiting = 「{ $name }」等你确认操作
digest-queued = 「{ $name }」排队中
digest-intro = 这是从主对话分出来的话题「{ $name }」。
digest-preceding-header = 主对话的片段（仅供参考）：
digest-board-header = 当前工作板：
digest-no-others = 暂无其他会话
digest-rules-line = 分工规矩：{ $rules }
digest-rules = 只负责这个话题里用户要的事；主对话和其他话题在做的事不要接着做；要碰它们正在改的文件，先停下问用户。
digest-first = 用户的第一句：{ $root }
digest-done = 话题「{ $name }」：已完成
digest-done-files = 话题「{ $name }」：已完成，改了 { $files } 个文件
digest-done-own = 「{ $name }」已完成
digest-done-summary = 「{ $name }」已完成：{ $summary }
digest-files-count = ，改了 { $files } 个文件
digest-files-list = ，改了：{ $files }
digest-executed = 「{ $name }」执行了「{ $title }」
digest-conflict = 「{ $first }」和「{ $second }」都改了 { $path }（事后发现）
digest-ended = 「{ $name }」已结束（{ $reason }）
digest-board-line = 「{ $name }」{ $status }
digest-board-line-doing = 「{ $name }」{ $status }：{ $doing }
topic-exec-default = 命令
topic-no-ask-mode = 这个 Agent 没有「改前先问」的模式（没配置 WONTON_ACP_TOPIC_MODE 或这个 Agent 不认），改文件前不会问，只能事后提醒冲突。
