## Agent 进度状态消息（ADR-0082）：step 和 body 只按工具类别写，不带标题、参数、路径。

run-thinking = 在想
run-replying = 在写回复
run-tool-read = 在读文件
run-tool-edit = 在改文件
run-tool-delete = 在删除
run-tool-move = 在移动
run-tool-search = 在搜索
run-tool-execute = 在执行命令
run-tool-fetch = 在上网
run-tool-other = 在忙别的
run-waiting = 等你确认
run-done-steps = 完成，用了 { $steps } 步
run-done-plain = 完成
run-cancelled = 打断了
run-max-tokens = 失败：回复达到输出长度上限
run-max-turns = 失败：达到轮次上限
run-refusal = 失败：Agent 拒绝了请求
run-failed-other = 失败：{ $reason }
run-uncertain = 结果不确定
run-bridge-restart = 桥重启了，这一轮结果不确定
