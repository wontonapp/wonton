# chat-issue-814：两端编辑消息的壳测试和截图

协议和核心已经合了（#802 的编辑语义、#804 的接线），两端菜单里的“编辑”、输入框上方的编辑条、确认/取消都已经在 main 上。这轮补**两端的界面测试和浅深截图**，顺手修测试照出来的问题。

卡：chat-issue-814。工作目录就是这一份，分支 `feat/edit-message-shells`，从最新 `main` 切。

## 先看

- `apps/ios/Wonton/TimelineRow.swift:19,227`（菜单“编辑”，`messageEdit` 标识）、`apps/ios/Wonton/ComposerView.swift:430` 起的编辑条、`apps/ios/Wonton/ChatModel+Editing.swift`（`startEdit`/`cancel` 路径）。
- `apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/MessageMenu.kt`、`ComposerEdit.kt`。
- ADR-0109 第 10 节；假数据里消息的 `edited`、`is_own`、失败态。

## 要做

每端两条界面用例，浅深都拍：

1. **能编辑的自己消息**：长按 → 菜单里有“编辑” → 点了进编辑态：输入框上方出现编辑条（铅笔、“编辑消息”、原正文摘要、关闭叉），发送键变成确认。取消：编辑条没了，输入框回到进编辑前的草稿，消息正文没变。
2. **不能编辑的消息**（别人的、或者已撤回的）：长按 → 菜单里没有“编辑”。

再各一条真发：

3. 确认成功：改了正文 → 确认 → 输入框清空、编辑条收起、消息正文变成新文字、`edited` 标记出现。

要求：

- 用例必须**修前能红**：现在没有这些用例，写好先跑一遍；照出来的壳问题（例如编辑条遮住东西、确认后草稿没恢复、菜单没收起）就地修，修完再跑绿。
- 截图放 `.walk/shots-<stamp>/`，浅深各一张：编辑态整屏、发成功后的消息。人眼看过再留。
- Android 用 `android-build.sh`（别手拼 cargo-ndk），iOS 用 `mini-walkthrough.sh`；`tests.env` 里只放这一轮的用例。

## 别做

- 不改协议和核心，除非测试证明核心有洞（有就写进 PR 的“底座的问题”，不要顺手改语义）。
- 不删除既有用例，不写只数行数的断言。

## 交付

- 分支 `feat/edit-message-shells` 开 PR（标题 `test(edit): 两端编辑消息的界面用例和截图`），正文写跑了什么、红过什么、修了什么。
- 卡上 `task.sh log chat-issue-814` 写结论（PR 号、红→绿证据、文件行号），然后 `.ready`。
- 不合并、不建 tag。
