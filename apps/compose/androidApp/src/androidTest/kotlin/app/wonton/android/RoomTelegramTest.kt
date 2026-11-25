package app.wonton.android

import android.text.StaticLayout
import android.text.TextPaint
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import kotlinx.coroutines.runBlocking
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.R
import java.text.DateFormat
import kotlin.math.abs
import org.junit.Test

/** 自己发的、发送失败的那条（`ui_fake_room.rs` 的 `FAILED_SEND_INDEX`）。 */
private const val FAILED_SEND = 24
/** 带一个发送失败的选项的那条（`ui_fake_room.rs` 的 `FAILED_CHOICE_INDEX`）。 */
private const val FAILED_CHOICE = 25
/** 失败原因的原文（`ui_fake_room.rs`）。 */
private const val FAILED_REASON = "Server timed out while sending"

/** 时间线本体（ADR-0109）：左滑回复、引用跳转、日期分隔、分组形状、选项按钮。 */
class RoomTelegramTest : RoomTelegramCase() {
    @Test
    fun swipeLeftStartsReply() {
        scrollTo(15)
        item(15).performTouchInput {
            val middle = height / 2f
            swipe(Offset(width * 0.8f, middle), Offset(width * 0.3f, middle), durationMillis = 300)
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
        screenshot("swipe-reply")
    }

    @Test
    fun quotedMessageJumpsAndBackButtonReturns() {
        scrollTo(21)
        item(21).performScrollTo()
        // 引用那块本身可点，问主人要不要跳的是它里面的文字。
        compose.onNodeWithText("Fixture message 5", useUnmergedTree = true).performClick()
        compose.waitForIdle()
        screenshot("after-quote-click")
        compose.waitUntil(10_000) { visible(5) && !visible(21) }
        screenshot("jump-highlight")
        // 离开底部时“向下”先退回跳之前的那条，不直接回最新。
        compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom), substring = true).performClick()
        compose.waitUntil(10_000) { visible(21) }
        screenshot("jump-back")
    }

    @Test
    fun quoteBlockHugsItsContent() {
        scrollTo(LONG_REPLY)
        item(LONG_REPLY).performScrollTo()
        val quote = compose.onNode(
            hasTestTag("quote") and hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        )
        val box = quote.fetchSemanticsNode().boundsInRoot
        val name = compose.onNode(
            hasText("Owner") and hasAnyAncestor(hasTestTag("quote")) and
                hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot
        val summary = compose.onNode(
            hasText(QUOTED_TEXT.trim()) and hasAnyAncestor(hasTestTag("quote")) and
                hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot
        val slack = with(compose.density) { 12.dp.toPx() }
        // 引用块只占名字加内容本身那么高：内容就是两行，多出来的就是被拉高的。
        check(summary.bottom >= name.bottom) { "引用块的正文跑到名字上面：${summary.bottom} < ${name.bottom}" }
        check(box.height <= name.height + summary.height + slack) {
            "引用块被拉高：${box.height} > 名字 ${name.height} + 内容 ${summary.height} + $slack"
        }
        check(box.bottom - summary.bottom <= slack) { "引用块底下多出 ${box.bottom - summary.bottom}" }
    }

    @Suppress("CyclomaticComplexMethod") // 每条消息的定位、行框、占位裁切和落点判断都在这里，摊平反而要来回跳。
    @Test
    fun timeNeverCoversMessageText() {
        scrollTo(LONG_REPLY)
        item(LONG_REPLY).performScrollTo()
        fun timeNode(index: Int) = compose.onAllNodes(
            hasTestTag("messageTime") and hasAnyAncestor(hasTestTag("item-ui-fake-$index")),
            useUnmergedTree = true,
        ).fetchSemanticsNodes().singleOrNull()
        for (index in 18..23) {
            // 一条一条滚进来：离屏的条目语义在、框是空的。
            scrollTo(index)
            compose.waitForIdle()
            var time = timeNode(index)
            if (time != null && time.boundsInRoot.width == 0f) {
                compose.onNodeWithTag("room-timeline").performTouchInput { swipeUp() }
                compose.waitForIdle()
                time = timeNode(index)
            }
            if (time == null || time.boundsInRoot.width == 0f) continue
            val box = time.boundsInRoot
            val bubble = bubble(index).fetchSemanticsNode().boundsInRoot
            check(bubble.contains(box.topLeft) && bubble.contains(box.bottomRight - Offset(1f, 1f))) {
                "时间跑出气泡：$box 不在 $bubble"
            }
            val nodes = bodyNodes(index).sortedBy { it.boundsInRoot.top }
            // 末尾可能接着旁聊卡片，它不带占位：从后往前找最后一段带透明占位的正文。
            fun reserveOf(node: SemanticsNode) = layoutOf(node)?.layoutInput?.text?.spanStyles
                ?.firstOrNull { it.item.color == Color.Transparent }
            val reserve = nodes.lastOrNull { reserveOf(it) != null }?.let(::reserveOf)
            val tailNode = nodes.lastOrNull { reserveOf(it) != null } ?: nodes.last()
            val layout = layoutOf(tailNode)!!
            val tail = layout.layoutInput.text.text
            // 末尾那段透明占位是单独一段透明色的样式接在最后一块字后面，从它的起点切开就不算正文；
            // 卡片条目的末节点是按钮，不带占位。不按空格或 NBSP 认：时间里有普通空格，回执才是 NBSP。
            val keep = reserve?.start ?: tail.length
            val lines = nodes.flatMap { lineBoxes(it, if (it === tailNode) tail.length - keep else 0) }
            check(lines.isNotEmpty()) { "第 $index 条没有正文行框：'$tail'" }
            val hit = lines.firstOrNull { it.overlaps(box) }
            check(hit == null) {
                "时间压在第 $index 条的字上：$box 与 $hit；气泡 $bubble；" +
                    nodes.joinToString(" | ") { "${it.boundsInRoot} '${layoutOf(it)!!.layoutInput.text.text}'" }
            }
            // 没有占位的消息（删除的）脚注本来就单起一行，只查不压字。
            if (reserve != null) assertTimeOnLastLine(tailNode, tail, keep, box)
        }
    }

    /** 时间得压在最后一行上（占位自己折到新行时脚注跟着落在新行也对）；占位没折行或丢了还掉到下面就不算。 */
    private fun assertTimeOnLastLine(tailNode: SemanticsNode, tail: String, keep: Int, box: Rect) {
        val textLines = lineBoxes(tailNode, tail.length - keep)
        val lastLine = textLines.lastOrNull() ?: error("没有正文行框：'$tail'")
        val wrapped = lineBoxes(tailNode, 0).size > textLines.size
        check(wrapped || box.top < lastLine.bottom) { "时间没落在最后一行：$box 与 $lastLine" }
    }

    @Test
    fun longReplyKeepsEveryParagraphWhole() {
        scrollTo(LONG_REPLY)
        item(LONG_REPLY).performScrollTo()
        val nodes = bodyNodes(LONG_REPLY).sortedBy { it.boundsInRoot.top }
        check(nodes.size == LONG_REPLY_PARAGRAPHS.size) { "段落少了：${nodes.size} 段" }
        for ((node, expected) in nodes.zip(LONG_REPLY_PARAGRAPHS)) {
            val layout = layoutOf(node) ?: continue
            val drawn = layout.layoutInput.text.text
            check(drawn.startsWith(expected)) { "这一段的文字对不上：'$drawn'" }
            // 语义里要留着最后一句，读屏和复制拿到的都是全文。
            check(drawn.contains(expected.takeLast(6))) { "这一段少了结尾：'${drawn.takeLast(12)}'" }
            // 换行数按同样的宽度重新排一遍；少一行就是被截断了。
            val paint = TextPaint().apply {
                textSize = with(compose.density) { layout.layoutInput.style.fontSize.toPx() }
            }
            val needs = StaticLayout.Builder.obtain(expected, 0, expected.length, paint, node.size.width)
                .build().lineCount
            check(layout.lineCount >= needs - 1) { "这一段被截断：画了 ${layout.lineCount} 行，要 $needs 行" }
        }
        screenshot("long-reply")
    }

    @Test
    fun choiceAnswerSelectsItAndDisablesTheOther() {
        scrollTo(22)
        val approve = choice("✅")
        val deny = choice("❌")
        approve.assertIsEnabled()
        // 表情先画出来，按钮行不比气泡宽。
        compose.onAllNodesWithText("✅", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        val rowWidth = compose.onNode(
            hasTestTag("choices-row") and hasAnyAncestor(hasTestTag("item-ui-fake-22")),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot.width
        val bubbleWidth = bubble(22).fetchSemanticsNode().boundsInRoot.width
        check(rowWidth <= bubbleWidth + 0.5f) { "按钮行比气泡宽：$rowWidth > $bubbleWidth" }
        // 表情和字在同一行：两个按钮一样高，说明不比同行的表情高多少（折行会翻倍）。
        val heights = listOf(approve, deny).map { it.fetchSemanticsNode().boundsInRoot.height }
        check(abs(heights[0] - heights[1]) <= 1f) { "两个按钮不一样高：$heights" }
        val emoji = compose.onAllNodesWithText("✅", useUnmergedTree = true).fetchSemanticsNodes().single()
        val label = compose.onAllNodesWithText("approve once", useUnmergedTree = true).fetchSemanticsNodes().single()
        check(label.boundsInRoot.height <= emoji.boundsInRoot.height * 1.5f) {
            "说明折行了：${label.boundsInRoot.height} > ${emoji.boundsInRoot.height}"
        }
        check(label.boundsInRoot.top < approve.fetchSemanticsNode().boundsInRoot.bottom)
        // 表情和字之间留一点缝（s8），别贴成一个词，也别拉成两截。
        val want = with(compose.density) { 8.dp.toPx() }
        val gap = label.boundsInRoot.left - emoji.boundsInRoot.right
        check(abs(gap - want) <= 2f) { "表情和字之间是 $gap，不是 $want" }
        screenshot("choices")
        approve.performClick()
        compose.waitUntil(10_000) { !deny.isEnabled() }
        choice("✅").assertIsSelected()
        deny.assertIsNotEnabled()
        screenshot("choice-answered")
    }

    @Test
    fun groupHeadAndTailShapes() {
        openFamily()
        // 群里的别人有头像，私聊没有。
        compose.waitUntil(10_000) { avatars().isNotEmpty() }
        scrollTo(8)
        item(8).performScrollTo()
        item(8).assertIsDisplayed()
        screenshot("group-head")
        scrollTo(10)
        item(10).performScrollTo()
        item(10).assertIsDisplayed()
        // 组尾：尾巴那条和组内上一条的气泡左边对齐，头像挂在组尾。
        val head = bubble(9).fetchSemanticsNode().boundsInRoot.left
        check(bubble(10).fetchSemanticsNode().boundsInRoot.left == head) { "组内气泡左边没对齐" }
        screenshot("group-tail")
    }

    @Test
    fun groupShotShowsNameAndAvatar() {
        openFamily()
        scrollTo(8)
        item(8).performScrollTo()
        scrollTo(10)
        item(10).assertIsDisplayed()
        compose.waitUntil(10_000) { avatars().isNotEmpty() }
        screenshot("group")
    }

    @Test
    fun directRoomHasNoSenderAvatar() {
        scrollTo(8)
        item(8).performScrollTo()
        compose.onAllNodes(hasTestTag("sender-avatar"), useUnmergedTree = true).assertCountEquals(0)
    }

    @Test
    fun groupTopKeepsFirstBubbleBelowTheBar() {
        openFamily()
        // 26 是最早的一条，顶到列表头上就是它。
        scrollTo(26)
        repeat(3) {
            compose.onNodeWithTag("room-timeline").performTouchInput {
                swipe(Offset(centerX, centerY * 1.9f), Offset(centerX, centerY * 0.3f), durationMillis = 400)
            }
            compose.waitForIdle()
        }
        bubble(26).assertIsDisplayed()
        // 最早那条气泡的顶边还在顶栏下面，没被顶栏拿走。
        val first = bubble(26).fetchSemanticsNode().boundsInRoot
        // 顶栏和它下面那行（群里的 AI 等）都不能压在第一条上。
        val bar = compose.onNodeWithTag("room-bar").fetchSemanticsNode().boundsInRoot.bottom
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot.bottom
        val limit = maxOf(bar, header)
        check(first.top >= limit) { "第一条被顶栏盖住：${first.top} < $limit" }
    }

    @Test
    fun pluginStatusRowCoversNoBubble() {
        // 群里那行 AI（假房间里是 Hermes）自己占一条，时间线让开它，气泡不压在它上面。
        openFamily()
        compose.onNodeWithText("Hermes", useUnmergedTree = true).assertIsDisplayed()
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true).fetchSemanticsNodes()
        check(bubbles.isNotEmpty())
        for (bubble in bubbles) {
            val box = bubble.boundsInRoot
            check(!box.overlaps(header)) { "气泡压在状态行上：$box vs $header" }
        }
    }

    @Test
    fun datePillFadesAfterScrollingStops() {
        scrollTo(15)
        compose.onNodeWithTag("room-timeline").performTouchInput {
            swipe(Offset(centerX, height * 0.7f), Offset(centerX, height * 0.2f), durationMillis = 300)
        }
        compose.waitUntil(5_000) { pillVisible() }
        // 停下 motion.datePillHold（1 秒）后淡出，等不到就是没淡出。
        compose.waitUntil(10_000) { !pillVisible() }
    }

    @Test
    fun daySeparatorRidesAboveTheFirstOfADay() {
        // 第 11 条是后一天的第一条：分隔夹在它和上面第 10 条之间，写它的日期。分隔翻到附近才进屏幕，撞上为止。
        val (separators, bubbles) = scrollToFirstSeparator()
        for (separator in separators) separatorIsAboveTheOneItNames(separator, bubbles)
        noSeparatorBetweenTheSameDay()
        // 屏幕上往下是越来越新：分隔自上而下的日期严格变晚。
        val dates = separators.sortedBy { it.boundsInRoot.top }
            .map { checkNotNull(DateFormat.getDateInstance(DateFormat.MEDIUM).parse(texts(it))) }
        for (position in 1 until dates.size) {
            check(dates[position].after(dates[position - 1])) { "日期没随消息往下变晚：$dates" }
        }
        // 列表最顶上那条的上面也有分隔，写它自己的日期。分隔本体在它上面一点，滚到房间开头才进得了屏幕。
        compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-start")
        compose.waitForIdle()
        val firstOfList = bubble(0).fetchSemanticsNode()
        val aboveFirst = compose.onAllNodes(daySeparator(), useUnmergedTree = true)
            .fetchSemanticsNodes()
            .lastOrNull { it.boundsInRoot.bottom <= firstOfList.boundsInRoot.top + 1f }
        checkNotNull(aboveFirst) { "列表最顶上那条的上面没有日期分隔" }
        check(texts(aboveFirst) == fixtureDate(0)) { "最顶上的分隔不是第一条的日期：${texts(aboveFirst)}" }
        // 这条分隔和它两边的消息一起进画面，留一张给人看。
        scrollTo(11)
        compose.waitUntil(10_000) { visible(10) && visible(11) }
        screenshot("day-separator")
    }

    /** 翻到有分隔的地方，返回分隔和气泡（按屏幕从上到下）。 */
    private fun scrollToFirstSeparator(): Pair<List<SemanticsNode>, List<SemanticsNode>> {
        var separators = emptyList<SemanticsNode>()
        var seen = ""
        for (index in listOf(11, 12, 10, 13, 9)) {
            scrollTo(index)
            compose.waitForIdle()
            separators = compose.onAllNodes(daySeparator(), useUnmergedTree = true)
                .fetchSemanticsNodes()
            seen += "$index:${separators.size} "
            if (separators.isNotEmpty()) break
        }
        check(separators.isNotEmpty()) { "没画日期分隔：$seen" }
        return separators to compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true)
            .fetchSemanticsNodes().sortedBy { it.boundsInRoot.top }
    }

    /** 日期分隔：核心在跨日处发的分隔条目（`day-divider`），或壳兜底画的那一行（`day-separator`）。 */
    private fun daySeparator() = hasTestTag("day-separator") or hasTestTag("day-divider")

    /** 分隔夹在前一天最后一条和后一天第一条之间：下面紧挨的那条是它写的这一天，上面那条是更早的一天。 */
    private fun separatorIsAboveTheOneItNames(separator: SemanticsNode, bubbles: List<SemanticsNode>) {
        val mark = separator.boundsInRoot
        check(bubbles.none { it.boundsInRoot.overlaps(mark) }) { "气泡压在日期分隔上：$mark" }
        val below = bubbles.firstOrNull { it.boundsInRoot.top >= mark.bottom - 1f }
        val belowIndex = checkNotNull(below?.let(::fixtureIndexOf)) { "日期分隔下面认不出气泡：$mark" }
        check(texts(separator) == fixtureDate(belowIndex)) {
            "日期分隔不是它下面那条的日期：${texts(separator)} vs ${fixtureDate(belowIndex)}（第 $belowIndex 条）"
        }
        val above = bubbles.lastOrNull { it.boundsInRoot.bottom <= mark.top + 1f } ?: return
        val aboveIndex = fixtureIndexOf(above) ?: return
        val aboveBox = above.boundsInRoot
        check(fixtureDate(aboveIndex) != fixtureDate(belowIndex)) {
            "同一天（第 $aboveIndex / $belowIndex 条）的气泡之间夹了日期分隔"
        }
        val order = "分隔上面第 $aboveIndex 条 $aboveBox，" +
            "下面第 $belowIndex 条 ${below.boundsInRoot}，分隔 $mark"
        check(fixtureDate(aboveIndex) < fixtureDate(belowIndex)) { "上面的日期不早于下面：$order" }
    }

    /** 同一天相邻的两条之间没有分隔。 */
    private fun noSeparatorBetweenTheSameDay() {
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true)
            .fetchSemanticsNodes().sortedBy { it.boundsInRoot.top }
        val separators = compose.onAllNodes(daySeparator(), useUnmergedTree = true)
            .fetchSemanticsNodes()
        val sameDay = bubbles.zipWithNext().filter { (above, below) ->
            val a = fixtureIndexOf(above)
            val b = fixtureIndexOf(below)
            a != null && b != null && fixtureDate(a) == fixtureDate(b)
        }
        for ((above, below) in sameDay) {
            val between = separators.filter {
                it.boundsInRoot.top >= above.boundsInRoot.bottom - 1f &&
                    it.boundsInRoot.bottom <= below.boundsInRoot.top + 1f
            }
            val pair = "${fixtureIndexOf(above)} / ${fixtureIndexOf(below)}"
            check(between.isEmpty()) { "同一天的第 $pair 条之间夹了日期分隔" }
        }
    }

    @Test
    fun barTitleAndAvatarOpenBotProfile() {
        val profile = compose.onNodeWithTag("bot-profile")
        // 入口只剩头像一个，顶栏里没有额外的信息按钮。
        compose.onAllNodesWithContentDescription(string(R.string.bot_profile_open)).assertCountEquals(1)
        compose.onNodeWithTag("roomTopBarTitle").performClick()
        profile.assertIsDisplayed()
        // 资料页自己的返回按钮跟页面一起出现；系统返回按早了会把整个 Activity 关掉。
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithTag("roomTopBarAvatar").performClick()
        profile.assertIsDisplayed()
        screenshot("bar-profile")
    }

    @Test
    fun failedChoiceKeepsReasonInDetails() {
        scrollTo(FAILED_CHOICE)
        item(FAILED_CHOICE).performScrollTo()
        // 按钮里不再画原因，原因在下面一行小字里。
        compose.onAllNodes(hasText(FAILED_REASON) and hasAnyAncestor(hasTestTag("choice-⏳")), useUnmergedTree = true)
            .assertCountEquals(0)
        compose.onNodeWithText(string(R.string.message_choice_failed)).assertIsDisplayed()
        compose.onNodeWithText(string(R.string.common_details)).performClick()
        compose.onNodeWithText(FAILED_REASON).assertIsDisplayed()
        screenshot("failed-choice-open")
        // 失败的按钮还能点，点了就重发。
        choice("⏳").assertIsEnabled().performClick()
        compose.waitUntil(10_000) {
            choice("⏳").fetchSemanticsNode().config[SemanticsProperties.Selected]
        }
        compose.waitUntil(10_000) { compose.onAllNodesWithText(FAILED_REASON).fetchSemanticsNodes().isEmpty() }
        screenshot("failed-choice-resent")
    }

    @Test
    fun failedMarkHasATargetSizedHitArea() {
        scrollTo(FAILED_SEND)
        item(FAILED_SEND).performScrollTo()
        val mark = compose.onNodeWithContentDescription(string(R.string.timeline_send_failed_retrying))
        val target = with(compose.density) { 48.dp.toPx() }
        val box = mark.fetchSemanticsNode().boundsInRoot
        check(box.width >= target - 1f) { "失败标记宽 ${box.width}，不到 $target" }
        check(box.height >= target - 1f) { "失败标记高 ${box.height}，不到 $target" }
        // 整个命中区都能点开重发菜单。
        mark.performClick()
        compose.onNodeWithText(string(R.string.timeline_resend)).assertIsDisplayed()
        screenshot("failed-mark-menu")
    }

    @Test
    fun confirmationBarNamesTheCardAndOpensIt() {
        val bar = compose.onNodeWithTag("room-confirmations")
        bar.assertIsDisplayed()
        val target = with(compose.density) { 48.dp.toPx() }
        check(bar.fetchSemanticsNode().boundsInRoot.height >= target - 1f) { "确认入口不到 48dp 高" }
        // 写清是哪一张：一行“有事要你定”，下一行是首条的标题。
        compose.onNode(
            hasText(string(R.string.confirmation_card_requests)) and hasAnyAncestor(hasTestTag("room-confirmations")),
            useUnmergedTree = true,
        ).assertIsDisplayed()
        compose.onNodeWithText("Echo wants to install Home Hub", useUnmergedTree = true).assertIsDisplayed()
        screenshot("room-confirmations")
        bar.performClick()
        compose.onNodeWithText("Home Hub can read and write everything on the disks you tick.")
            .assertIsDisplayed()
        screenshot("room-confirmations-open")
    }


    /**
     * 回形针打开相册网格，点两张以后底下换成附言条，写了附言点发送：面板收起、附件区不留东西，
     * 时间线里出现带这张附言的消息。相册里没有照片时先放两张进去；照片只在本机相册里。
     * 附件区存在盘上、进程之间留着：先把剩下的拿掉。
     */
    @Test
    fun attachSheetSendsPhotosWithNote() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        ensureAlbumPhotos()
        clearTray()
        compose.onNodeWithContentDescription(string(R.string.composer_attach)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("album-grid")).fetchSemanticsNodes().size == 1 }
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("album-photo")).fetchSemanticsNodes().size >= 2 }
        compose.onAllNodes(hasTestTag("album-photo"))[0].performClick()
        compose.onAllNodes(hasTestTag("album-photo"))[1].performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isNotEmpty() }
        val field = compose.onNode(hasSetTextAction() and hasAnyAncestor(hasTestTag("attach-send")))
        field.performTextInput("两张都发")
        compose.waitForIdle()
        val shown = compose.onAllNodes(hasText("两张都发", substring = true)).fetchSemanticsNodes().size
        check(shown > 0) { "输入框里没进去：${field.fetchSemanticsNode().config}" }
        screenshot("attach-sheet")
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty()
        }
        runCatching {
            compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom), substring = true)
                .performClick()
        }
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasText("两张都发", substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
        val vm = runBlocking { core.subscribeComposer(TimelineTarget(ECHO_ROOM, null)).use { it.next()!! } }
        check(vm.attachments.isEmpty()) { "附件区还剩 ${vm.attachments.size} 条" }
        screenshot("attach-sent")
        check(trayTiles() == 0) { "附件区里还剩着照片" }
        compose.onNode(hasSetTextAction()).assertIsDisplayed()
    }

    /**
     * 附件区里有一张没传上去的照片时点发送：核心拒收，字、附件和回复对象都得留着，原因说出来。
     * 现在发送一律当成功，字丢了、回复也收了。
     */
    @Test
    fun rejectedSendKeepsDraftAndReply() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        ensureAlbumPhotos()
        clearTray()
        // 面板选完就发，先在核心那边定好：下一张进附件区的照片上传失败。
        core.failNextAttachmentForTest(TimelineTarget(ECHO_ROOM, null))
        compose.onNodeWithContentDescription(string(R.string.composer_attach)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("album-photo")).fetchSemanticsNodes().isNotEmpty() }
        compose.onAllNodes(hasTestTag("album-photo"))[0].performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isNotEmpty() }
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty()
        }
        compose.waitUntil(10_000) { trayTiles() == 1 }
        // 回一条，附件已经在附件区里标成失败了。
        scrollTo(15)
        item(15).performTouchInput {
            val middle = height / 2f
            swipe(Offset(width * 0.8f, middle), Offset(width * 0.3f, middle), durationMillis = 300)
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
        compose.onNode(hasSetTextAction()).performTextInput("先发这句")
        compose.onNodeWithContentDescription(string(R.string.composer_send)).performClick()
        // 被拒的原因写在弹窗里，点“确定”收掉。
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasText("Retry or remove it first", substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText(string(R.string.design_ok)).performClick()
        compose.onNode(hasText("先发这句")).assertIsDisplayed()
        check(trayTiles() == 1) { "附件被吃掉了" }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
    }
}
