package app.wonton.android

import android.text.StaticLayout
import android.text.TextPaint
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.test.swipeUp
import java.text.DateFormat
import org.junit.Test

/** 时间线排版（ADR-0109 第 3、4 节）：时间和正文互不遮挡、长段不丢字、日期胶囊和日期分隔。 */
class RoomTelegramTimelineLayoutTest : RoomTelegramCase() {
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
}
