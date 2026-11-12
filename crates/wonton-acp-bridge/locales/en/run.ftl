## Agent 进度状态消息（ADR-0082）：step 和 body 只按工具类别写，不带标题、参数、路径。

run-thinking = Thinking
run-replying = Writing the reply
run-tool-read = Reading files
run-tool-edit = Editing files
run-tool-delete = Deleting files
run-tool-move = Moving files
run-tool-search = Searching
run-tool-execute = Running a command
run-tool-fetch = Browsing the web
run-tool-other = Working
run-waiting = Waiting for you
run-done-steps = Done, { $steps } steps
run-done-plain = Done
run-cancelled = Interrupted
run-max-tokens = Failed: the reply hit the output length limit
run-max-turns = Failed: the turn limit was reached
run-refusal = Failed: the agent refused
run-failed-other = Failed: { $reason }
run-uncertain = Result uncertain
run-bridge-restart = The bridge restarted; the result of this turn is uncertain
