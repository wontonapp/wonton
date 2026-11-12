## 话题会话（ADR-0058）发进聊天和拼给 Agent 的文字。Agent 读的开局、动态也用这里的语言，
## 跟房间主人一致。

name-main = Main conversation

topic-interrupt-pending = Busy: an interrupt is already waiting.
topic-no-active-session = No active ACP session
topic-steer-rejected = The agent did not accept the steer request.
topic-queue-full = The ACP queue is full; try again later.
topic-input-too-long = Input is over the { $size } byte limit
topic-input-empty = Input can't be empty
topic-control-no-text = The control command is missing its text
topic-session-lost = The ACP session was lost and a new one was started; the earlier context is gone.
topic-queued = Topic queued: { $ahead } topics are active now; it starts when one of them ends.
topic-permission-failed = Agent permission request failed: { $message }
topic-permissions-cancelled = Permission request was cancelled; the agent did not run the operation.
topic-output-limited = Upstream error: the agent reply was over the output limit, so this turn was cancelled.
topic-no-text = The agent produced no text this turn (stopReason={ $reason }).
topic-idle-ended = The topic was idle for over 30 minutes, so it ended.
topic-main-cannot-end = The main conversation can't be ended.
topic-already-ended = This topic has already ended.
topic-ended-running = The topic ended; this turn is still running and its result goes to this topic.
topic-ended = Topic ended.
topic-ended-idle = idle for over 30 minutes
topic-ended-manual = ended by the user

status-working = Working
status-waiting = Waiting for you
status-idle = Idle
status-finished = Finished
status-queued = Queued

conflict-reject-reason = 「{ $other }」is editing { $path }; the user chose not to change it now
conflict-reject-reason-short = The user chose not to change it
conflict-approved = 「{ $name }」finished this turn; changes to { $path } are approved.
conflict-in-use-title = { $title } （「{ $other }」 is editing it too）
conflict-wait-option = Wait for 「{ $other }」 to finish
conflict-now-option = Change it now
conflict-reject-option = Don't change it
conflict-in-use-note = 「{ $other }」is editing { $path }; changing it before that turn finishes can be overwritten.

digest-header = Other conversations:
digest-working = 「{ $name }」is doing: { $doing }
digest-waiting = 「{ $name }」is waiting for you to confirm an operation
digest-queued = 「{ $name }」is queued
digest-intro = This topic was split off the main conversation as 「{ $name }」.
digest-preceding-header = From the main conversation, for reference:
digest-board-header = Current board:
digest-no-others = No other conversations
digest-rules-line = Ground rules: { $rules }
digest-rules = Only do what the user asked in this topic; don't continue what the main conversation or other topics are doing; if you need a file they are editing, stop and ask the user first.
digest-first = The user's first message: { $root }
digest-done = Topic 「{ $name }」: done
digest-done-files = Topic 「{ $name }」: done, { $files } files changed
digest-done-own = 「{ $name }」finished
digest-done-summary = 「{ $name }」finished: { $summary }
digest-files-count = , { $files } files changed
digest-files-list = , changed: { $files }
digest-executed = 「{ $name }」ran 「{ $title }」
digest-conflict = 「{ $first }」and 「{ $second }」both changed { $path } (found afterwards)
digest-ended = 「{ $name }」ended ({ $reason })
digest-board-line = 「{ $name }」{ $status }
digest-board-line-doing = 「{ $name }」{ $status }: { $doing }
topic-exec-default = a command
topic-no-ask-mode = This agent has no ask-before-edit mode (WONTON_ACP_TOPIC_MODE is unset or the agent doesn't support it), so it won't ask before changing files; conflicts show up only afterwards.
