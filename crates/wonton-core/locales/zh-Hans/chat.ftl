## 家里的服务器发进聊天的文字（ADR-0043）：确认卡片和它的结果行、给其他 Matrix 客户端看的
## 纯文本卡片、Bot 房间交不出去时的踢出理由、Agent 转述给主人的帮助文字。一条完整的话一条
## 消息；数据（Bot 名字、应用名、版本、路径、上游的报错原文）作参数嵌进句子，不翻。

# 确认卡片上的按钮。
chat-choice-install = 安装
chat-choice-deny-install = 不装
chat-choice-update = 更新
chat-choice-deny-update = 不更新
chat-choice-uninstall = 卸载
chat-choice-cancel = 取消
chat-choice-allow = 允许
chat-choice-deny = 拒绝

# 工具确认卡片上的放行选项：一段时间内再调用不再问。参数是整数的分钟数或秒数。
chat-card-window-minutes = { $minutes } 分钟内都允许
chat-card-window-seconds = { $secs } 秒内都允许

# 安装和更新卡片上的行。$requester 是 Bot 的名字，$title 是应用的名字。
chat-card-image = 服务 { $service } 用镜像 { $image }
chat-card-ports-added = 新增局域网端口 { $ports }
chat-card-gpu-added = 新增用显卡的服务 { $services }
chat-card-version-official = 版本 { $version }，来自官方索引
chat-card-version = 版本 { $version }
chat-card-update-from = 从版本 { $from } 更新到 { $to }
chat-card-no-permissions = 不需要额外的权限
chat-card-permissions-header = 要授予的权限：
chat-card-agent-source = 来自你的 Agent · { $requester }，没有经过审核：代码由 Agent 写，你自己把关
chat-card-agent-ask-again = 以后 { $requester } 改这个应用不再问，除非要更多权限
chat-card-update-header-added = 这次更新新增的项：
chat-card-update-header-risky = 这次更新申请了很高或最高级的权限：
chat-card-update-header-ask-every = 权限没有变宽，你打开了“每次都问”：
chat-card-update-header-adopt = 这个应用是你装的，同意后交给 { $requester } 管：
chat-card-update-adopt-replace = 同意后换成它交来的这份，数据保留，装不上会退回

# 卡片标题。$requester 是 Bot 的名字；$title、$plugin、$tool 是应用给的名字。
chat-card-install-title = { $requester } 想安装 { $title }
chat-card-update-title = { $requester } 想更新 { $title }
chat-card-uninstall-title = { $requester } 想卸载 { $title }
chat-card-permission-enable-title = { $requester } 要在“{ $title }”上打开「{ $permission }」
chat-card-permission-disable-title = { $requester } 要在“{ $title }”上关掉「{ $permission }」
chat-card-permission-off-loss = 关掉以后：{ $loss }
chat-card-permission-root = 打开后这个应用能控制整台主机，操作以本机 root 身份执行。
chat-choice-permission-enable = 打开
chat-choice-permission-disable = 关掉
chat-permission-done = 已经改好“{ $title }”的权限
chat-permission-failed = 给 { $id } 改权限失败：{ $error }
chat-card-tool-title = { $requester } 想用 { $plugin } 的 { $tool }

# Agent 权限卡片的标题。$title 是 Agent 给这次请求起的名字。
chat-card-agent-title = { $requester }：{ $title }

# 最高级权限要多点一步的那张卡（ADR-0036）。
chat-card-highest-title = { $title } 能控制整台主机
chat-card-highest-line = 代码会以本机 root 身份运行
chat-card-highest-confirm = 我知道这个应用能控制整台主机

# 卸载卡片：应用的数据怎么处理。
chat-card-uninstall-purge = 并删除它的数据，和 Bot 的旧加密消息将无法再查看，不可恢复
chat-card-uninstall-keep = 保留它的数据，以后重装还在

# 工具调用卡片的行。$args 是原始 JSON 参数，太长时截断。
chat-card-tool-data = 这个工具会改动数据
chat-card-tool-root = 这个操作以本机 root 身份执行
chat-card-args = 参数：{ $args }

# Bot 存文件的卡片。$name 和 $size 描述一个文件；$total 是加在一起的大小。
chat-card-save-file = { $name }（{ $size }）
chat-card-save-more = 还有 { $count } 个文件
chat-card-save-total = 共 { $total }
chat-card-save-unreviewed = Bot 提供，内容未审核
chat-card-save-allow = 允许存进 { $folder }
chat-card-save-title = { $requester } 想把 { $count } 个文件存进 { $folder }

# 给其他 Matrix 客户端看的卡片正文。$message 是回答卡片时写的结果行，用主人的语言。
chat-card-state-pending = 请在云吞 App 里确认。
chat-card-state-running = 已确认，正在执行……
chat-card-state-succeeded = 已完成：{ $message }
chat-card-state-failed = 没有成功：{ $message }
chat-card-state-uncertain = 结果不确定：{ $message }
chat-card-state-rejected = 已拒绝，没有执行。
chat-card-state-expired = 没有在期限内确认，已过期，没有执行。

# 卡片正文下面关于放行时段的一行：还在生效，或者怎么结束的。$count 是放行时段里除第一次
# 之外放行的次数；0 表示不知道或没有。
chat-window-active = 放行时段生效中，到期前不再询问。
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] 放行时段已到期。
               *[other] 放行时段已到期，放行了 { $count } 次。
            }
        [stopped]
            { $count ->
                [0] 放行时段已停止。
               *[other] 放行时段已停止，放行了 { $count } 次。
            }
        [restarted] 服务器重启过，放行时段已结束。
       *[other]
            { $count ->
                [0] 放行时段已结束。
               *[other] 放行时段已结束，放行了 { $count } 次。
            }
    }

# 主人回答之后写上卡片的结果行。
chat-install-done = 已装好 { $title }
chat-install-done-url = 已装好 { $title }，网页地址 { $url }
chat-install-failed = 安装失败（{ $kind }）：{ $error }
chat-install-failed-id = 安装 { $id } 失败（{ $kind }）：{ $error }
chat-install-note-writer = 应用的归属没记下来（{ $error }），下次更新会装不上
chat-outcome-with-note = { $message }。{ $note }
chat-update-done = 已更新到 { $version }
chat-update-done-url = 已更新到 { $version }，网页地址 { $url }
chat-update-failed = 更新 { $id } 失败（{ $kind }）：{ $error }
chat-uninstall-done-purge = 已卸载 { $title }，数据已删除
chat-uninstall-done-keep = 已卸载 { $title }，数据保留
chat-uninstall-failed = 卸载 { $id } 失败（{ $kind }）：{ $error }
chat-tool-ran = 已执行
chat-tool-failed = 工具报告执行失败
chat-save-done = 已存进 { $folder }
chat-save-done-note = 已存进 { $folder }。{ $note }
chat-save-partial = 存好了 { $saved }，后面的没存成：{ $failure }
chat-save-note-grant = 授权没记下来（{ $error }），下次存还要问你
chat-agent-chose = 已选择“{ $choice }”
chat-agent-abandoned = Agent 不再等这个回答了，这次确认作废
chat-superseded = 已被新版本取代
chat-uncertain-restart = 服务器重启过，不确定有没有执行完

# 踢出理由：Bot 的房间交不到主人手里时，显示在房间的 timeline 里。
chat-kick-reason = 云吞没能建好这个私聊

# 帮助工具。Agent 读了这些再回答主人。$details 是上游给的原因；$required 和 $core 是版本号。
chat-help-docs-unavailable = 这台服务器没有帮助文档（{ $dir } 里没有，也没有取到过）
chat-help-refresh-failed = 取不到新的帮助文档：{ $details }
chat-help-refresh-needs-core = 有新的帮助文档，但服务器要更新到 { $required } 以上（这台是 { $core }）：先更新服务器
chat-help-docs-age = { $problem }。这是 { $days } 天前的文档
chat-help-topic-missing = 没有主题 { $topic }。可选的主题：{ $topics }
chat-help-page-needs-core = 这一页讲的功能要服务器更新到 { $required } 以上（这台是 { $core }）：先更新服务器

chat-mail-convert-failed = 这封信只有 HTML，正文没能转成文字，原文已作为 message.html 附上。
chat-mail-unknown-source = 收到一封来源不明或没有发信权限的应用邮件，未投递给任何人。
chat-mail-recipients-failed = 无法更新应用邮件的收件人：{ $error }。家人变动或服务器重启时会再试。
chat-notify-rate-limited = { $plugin } 发通知和信太频繁，已暂停，约 { $seconds } 秒后自动恢复。
# MCP Apps 界面里的调用在卡片上把调用方写成主人。
mcp-app-owner = 你

# Bot 往面板钉页（ADR-0052）。
panel-pin-card-title = { $bot } 想往你的面板里放页面
panel-pin-card-line = 同意后，这个 Bot 以后再往主面板放页面不再问；可以在设置里收回。
panel-pin-allowed = 已同意
panel-pin-bot-only = 只有 Bot 能钉页面；插件 { $plugin } 没有 Bot 身份
panel-pin-bot-missing = 找不到 Bot { $bot } 的记录
panel-pin-full = 面板已经满了（最多 { $max } 页），先拿掉一页再钉
panel-pin-already-asking = 已经有一张往主面板钉页的确认卡片在等主人回答
panel-pin-matrix = 连不上 Matrix：{ $error }
panel-pin-not-there = 这个页面没有钉在面板上

# MCP Apps 界面里的调用在卡片上把调用方写成主人。