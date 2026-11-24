# 复审 PR 939 增量 4（head 5474887）

- 作者自己把 b 轮三条改完推了 5474887390a1f7dd66dca25c7b6a6c368ab0d524（b 轮是 9569bfb）。按这个 head 重核三条：守卫、快照钉 UTC、三处 fixtureDate 断言。审查只读，改动都在本地分支 `review/939-5474887`，没推。
- PR 相对最新 main 冲突：`ui_fake_timeline.rs`（main #936 把转发编号从 `ui-fake-forwarded-{index}` 改成 `ui-fake-forwarded-{sent}`、条数断言改成 `ITEMS + 1`）和 `RoomTelegramTest.kt`（main 在命令菜单用例开头换成 `clearTray()`）。本地合并的取舍：
  - `ui_fake_timeline.rs`：取 main 的编号 `"ui-fake-forwarded-1"`，条数留 PR 的 `ITEMS + 4`（PR 加了 Start + 两条分隔共 3 条，`ITEMS` 现为 27）。
  - `RoomTelegramTest.kt`：取 main 的 `clearTray()`，保留 PR 的 `settleCommandMenu()`（popup 里收不到 IME insets 时要它兜）。
  - 合并后 `git grep` 无冲突标记。

## 三条都改了，改法对

- 守卫：`TimelineList.kt:309-311` 加了 `belowWritesDay`（下面紧挨着同一天的分隔条目时不出兜底行），和 b 轮给的写法一致；`index - 1` 为 -1 时 `getOrNull` 给 null，`own` 已排除空日期，不会误判。新增 `TimelineDayMarksTest.startAtTheTopRidesOnItsDaysDivider`（`[消息, 同日分隔, TimelineStart]` → `[null, null, null]`）。和真实顺序对得上：SDK 在每天第一条前面放分隔，翻到房间开头就是 `TimelineStart` → 分隔 → 第一条，正是被它盖掉的那一格；普通的跨天处那条分隔上面是前一天最后一条，本来就不出兜底行，不受影响。
- 快照：`dayBreak`/`dayBreakDark` 走新加的 `utcSnapshot`（`TimeZone.setDefault("UTC")` + finally 还原），`connectionDetailsFlows` 原来的内联时区逻辑收进同一个包装，没有第二处改时区的写法；两张 PNG 是 UTC 重录的（8:00 AM）。
- `CrossDayDateTest`：三处 `startsWith` 改成 `==`（`:88`、`:143`、`:154`），顶端改成滚到 `top-gap`、等 `startVisible()`，随后断言最顶上那行 tag 是 `day-divider`、写第一条的日期、第一条气泡上方没有别的日期行。正好是 b 轮指出的那两枚同日胶囊。

## 验证（本地合并后跑）

- `cargo check -p wonton-client --tests`：绿，0 warning。
- `cargo test -p wonton-client --features ui-fake ui_fake`：23 个全过（含 `forwarding_reaches_an_unopened_room`、`sent_text_reaches_an_unopened_room`、`two_subscriptions_share_one_room` 的条数断言）。
- `:shared:testAndroidHostTest --tests app.wonton.ui.TimelineDayMarksTest --tests app.wonton.ui.PreviewSnapshotsTest`：23 个全过（TimelineDayMarksTest 6、PreviewSnapshotsTest 17），Paparazzi 与已提交的两张 UTC 图逐像素相等。
- 模拟器（1080×2400，API 35）：装 Debug + androidTest，`CrossDayDateTest` → `OK (1 test)`（6.5 s），截图里跨天处只有一枚 `Oct 16, 2026`；自己滚到房间开头是 `Start of the chat` + 一枚 `Oct 13, 2026`，没有叠第二枚。
- 合并处沾到的用例：`RoomTelegramTest` 的 `commandMenuListsBotCommandsAndSends`、`forwardToSingleTargetOpensIt`、`forwardPickerSendsNoteBeforeMessageToEveryTarget`、`failedChoiceKeepsReasonInDetails`、`failedMarkHasATargetSizedHitArea`、`datePillFadesAfterScrollingStops` → `OK (6 tests)`。
- CI（run 45294601070，head 5474887）：android 的 detekt/单测/截图比对、rust 四片、Clippy、格式、假数据入口、gate 汇总全 pass。

## 观察（不进 VERDICT）

- `DayBreakFixtures` 还是用 `DateFormat` 出时间，靠 `utcSnapshot` 把时区钉住；只要这两张图继续走这个包装就不会漂，别把 `utcSnapshot` 单独从某个用例上拿掉。
- 1k 的 `emu-remote.sh free` 把 relay 单元也算成一个模拟器（`running_units` 只滤 `guard|tunnel-`，relay 是 `wonton-emu-relay-*`），有 relay 的模拟器在跑时它报 0 个空名额，`emu-slot.sh up` 直接判 1k 满。这轮是 `EMU_FORCE=1` 在本机起的。建议把 `relay` 加进过滤。

已评论：
复审通过 54748873 复审：rv-939e
VERDICT: PASS
