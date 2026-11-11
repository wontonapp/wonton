## 桥自己发进房间的状态话（ADR-0043）。一条完整的话一条消息；数据作参数，不翻。

login-reconnect = Agent 登录完成，正在重新连接。
input-unsupported = 输入不支持：只接受普通文字。
input-too-long = 输入非法：输入超过 { $max } 字节限制
input-empty = 输入非法：输入不能为空
control-without-text = 输入非法：控制命令缺少文字
interrupt-pending = 忙：已有一个打断请求在等待处理。
queue-full = 忙：ACP 队列已满，请稍后再试。
no-active-session = Agent 未就绪：没有活动 ACP session
steer-rejected = Agent 没有接受插话请求。
session-lost-new = ACP 会话已丢失，已新建会话；之前的上下文不再可用。
permission-failed = Agent 权限请求失败：{ $message }
permissions-cancelled = 权限请求已取消，Agent 没有执行该操作。
output-limited = 上游错误：Agent 回复超过输出限制，已取消本轮。
no-text = Agent 本轮没有产出文本（stopReason={ $stop_reason }）。
transport-disconnected = 传输不可用：ACP Agent 连接断开，正在重新连接。

catch-up-dropped =
    我刚才不在线，下面这 { $count } 条没有处理：
    { $list }
catch-up-dropped-more =
    我刚才不在线，下面这 { $count } 条没有处理：
    { $list }
    ……还有 { $rest } 条没有列出来

# 只进日志（日志不翻译，这几条和类别前缀连在一起读）。
agent-connect-socket = 连接 ACP socket { $path } 失败：{ $error }
agent-init-timeout = ACP initialize 超时
direct-task-failed = 私聊处理任务失败：{ $error }
stream-closed = Matrix 房间消息流已关闭
