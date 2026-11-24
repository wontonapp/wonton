# 复审 PR 939 增量 3（head 9569bfb）

- 上一轮 016a281 是 PASS；这个新提交自己说做的是“房间开头不再叠兜底日期行，补失败截图”，顺带把 `dayBreak`/`dayBreakDark` 从 `snapshotInUtc` 里拿了出来。审的就是 `9569bfb252dd8aef75ba4b9e479c3717ad6d9abe`（现取，等于本地 HEAD）。
- 增量 7 个文件：`CrossDayDateTest`、`RoomTelegramTest`、`PreviewSnapshotsTest`、`TimelineDayMarksTest`、`TimelineList`（只有注释）、`ui_fake_room`、`ui_fake_timeline`。

## 必须改：`dayMarksFor` 的守卫没写，CI 红

- KDoc 写了“下面紧挨着同一天的分隔条目时也不出”（`TimelineList.kt:295-296`），用例也按这个写（`TimelineDayMarksTest.kt:41-48`，期望 `[null, null, null]`），但**代码没动**：`TimelineList.kt:301-311` 还是原来的 `own && content !is DateDivider`，没有“下面是同一天的分隔条目”这一条。
- `TimelineStart` 没有时间（`timestampMs` 的 `else -> null`），在倒排列表里它是最后一项，`days.getOrNull(index + 1)` 为 null，于是 `own` 成立、它又不是 `DateDivider`，兜底行照出：屏幕顶上会是“兜底胶囊 → Start of the chat → 核心分隔胶囊（同一天）”，正是 rv-812 第 4 条要消掉的两枚。
- CI 已经红在这里：run 45257666370 / job 137984003352 第 10 步，`TimelineDayMarksTest > timelineStartAtTheTopLeansOnTheDividerBelow FAILED`，`java.lang.AssertionError at TimelineDayMarksTest.kt:47`，76 个用例 1 红（其余 75 个和 `:shared:detekt` 都过，所以是行为不对、不是写法问题）。
- 同一个原因，`CrossDayDateTest` 在设备上也会红：顶上最上面那行会变成 `day-separator` 的兜底行，`:88-90` 的 `day-divider` 断言不过；`dates` 严格变晚那条也会因为同一天出现两枚而挂。
- 改法（仍然一个跳转都不加）：

```kotlin
for (index in items.indices) {
    val day = days[index]
    val own = day != null && day != days.getOrNull(index + 1)
    val below = items.getOrNull(index - 1)
    val writtenBelow = below?.content is TimelineContent.DateDivider && days.getOrNull(index - 1) == day
    if (own && items[index].content !is TimelineContent.DateDivider && !writtenBelow) marks[index] = day
}
```

（`index - 1` 是屏幕上更靠下的那条；只有 `TimelineStart` 这种顶上、下面紧挨同日分隔的会命中。分隔条目本身不会走到这里。）

## 其它都对

- 假数据顶上加 `timeline_start()`（`ui_fake_room.rs:66-67, 236-241`）并占掉 `ITEMS + 3`，`ui_fake_room` 与 `ui_fake_timeline` 的四处偏移（`+2→+3`、`+3→+4`、`+6→+7`、`items[ITEMS + 3..]`）都对；`timeline_lands_away_from_a_tall_last_message` 仍取 `items.last()`（那还是待发消息，没被 start 顶掉）。
- `CrossDayDateTest` 换成滚到 `top-gap`、等 `startVisible()`（`TimelineList.kt:136` 的 spacer 是倒排列表最上面那项），逻辑对，等守卫补上就成立。
- `TestWatcher` 失败截图按 `RoomTelegramTest` 的写法加进规则链；`RoomTelegramTest` 去掉的那个空行、`CrossDayDateTest` 去掉的未用 import 都没问题。
- 产品代码这次只有一行注释，行为差异全在假数据和测试上。

## 观察（不进 VERDICT）

- `dayBreak`/`dayBreakDark` 从 `snapshotInUtc` 里拿出来了（`PreviewSnapshotsTest.kt:37,40`），但两张图还是 UTC 录的、`DayBreakFixtures` 仍用 `DateFormat`：CI（UTC）照旧相等，+8 机器会差 0.34%，在 1% 阈值内也能过。不是门槛；想保持“图随时区不漂”的话可以把这两个包回去（`snapshotInUtc` 还在，`connectionDetailsFlows` 在用）。

## 验证

- CI（run 45257666370）：`detekt、单元测试与 Debug 构建` 的 detekt/`:androidApp:testDebugUnitTest`/`assembleDebug` success，`比对界面参考图` 红在上面那个用例；rust 四片、假数据入口测试、Clippy、格式、lint 全 success。
- 增量没有改任何参考图（`git diff --stat 016a281 9569bfb` 无 PNG），Paparazzi 的图本身没差。
- 本地 `:shared:testAndroidHostTest --tests app.wonton.ui.TimelineDayMarksTest` 排上队后 CI 已给出同样结论，收掉了，没有本地结果。

已评论：https://github.com/wontonapp/wonton/pull/939#issuecomment-7113791695
复审要改 9569bfb 复审：rv-939b
VERDICT: CHANGES
