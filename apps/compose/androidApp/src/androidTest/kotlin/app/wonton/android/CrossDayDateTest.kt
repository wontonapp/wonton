package app.wonton.android

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToKey
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.send
import java.io.File
import java.io.FileOutputStream
import java.text.DateFormat
import java.util.Date
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description

/** 夜跑界面测试（#812）：跨天处只画一枚居中日期胶囊，写那一天，不叠第二行；浅色、深色各留一张现场。 */
@RunWith(AndroidJUnit4::class)
class CrossDayDateTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    // 不先给通知权限的话，系统的授权框盖在上面，界面不在前台。
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS)
    } else {
        GrantPermissionRule.grant()
    }

    private val failureEvidence = object : TestWatcher() {
        override fun failed(error: Throwable, description: Description) {
            screenshot("failure-${description.methodName}")
        }
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose).around(failureEvidence)

    @Before
    fun openEchoRoom() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.onNodeWithText("Echo")
            .performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    @Test
    fun oneDateLineAtTheCrossDay() {
        // 第 11 条是后一天的第一条：核心在那里发一条日期分隔条目，屏上只该有它这一行，没有另夹的兜底行。
        val lines = scrollToDateLines()
        for (line in lines) lineIsAboveTheOneItNames(line)
        oneLineAtTheDayBreak()
        noLineBetweenTheSameDay()
        // 屏幕上往下是越来越新：日期行自上而下的日期严格变晚。
        val dates = lines.sortedBy { it.boundsInRoot.top }.map { checkNotNull(parseDay(it)) }
        for (position in 1 until dates.size) {
            check(dates[position].after(dates[position - 1])) { "日期没随消息往下变晚：$dates" }
        }
        // 列表最顶上是房间开头的 TimelineStart，它下面紧挨着核心的分隔条目：顶端只该有分隔那一行，
        // 写第一条的日期，TimelineStart 和第一条之间不再叠兜底行。
        compose.onNodeWithTag("room-timeline").performScrollToKey("top-gap")
        compose.waitUntil(10_000) { startVisible() }
        val topLine = dateLines().minByOrNull { it.boundsInRoot.top }
        checkNotNull(topLine) { "列表最顶上那条的上面没有日期行" }
        check(texts(topLine) == fixtureDate(0)) { "最顶上那行不是第一条的日期：${texts(topLine)}" }
        check(topLine.config.getOrNull(SemanticsProperties.TestTag) == "day-divider") {
            "最顶上的日期行不是核心的分隔条目：${topLine.config.getOrNull(SemanticsProperties.TestTag)}"
        }
        val firstBubble = bubble(0).fetchSemanticsNode().boundsInRoot
        check(dateLines().none { it.id != topLine.id && it.boundsInRoot.bottom <= firstBubble.top + 1f }) {
            "最顶上那条上面又叠了一行：${dateLines().map(::texts)}"
        }
        // 这条分隔和它两边的消息一起进画面，留一张给人看；深色那张由 Paparazzi 的 DayBreakDarkPreview 出。
        scrollTo(11)
        compose.waitUntil(10_000) { visible(10) && visible(11) }
        screenshot("day-divider-light")
    }

    /** 翻到有跨日日期行的地方，返回它们（按屏幕从上到下）。 */
    private fun scrollToDateLines(): List<SemanticsNode> {
        var lines = emptyList<SemanticsNode>()
        var seen = ""
        for (index in listOf(11, 12, 10, 13, 9)) {
            scrollTo(index)
            compose.waitForIdle()
            lines = dateLines()
            seen += "$index:${lines.size} "
            if (lines.isNotEmpty()) break
        }
        check(lines.isNotEmpty()) { "没画日期行：$seen" }
        return lines.sortedBy { it.boundsInRoot.top }
    }

    /** 屏上的日期行：核心的分隔条目和兜底的分隔写的是同一件事，一起按一条日期行来看。 */
    private fun dateLines(): List<SemanticsNode> = compose.onAllNodes(
        SemanticsMatcher("是日期行") {
            when (it.config.getOrNull(SemanticsProperties.TestTag)) {
                "day-divider", "day-separator" -> true
                else -> false
            }
        },
        useUnmergedTree = true,
    ).fetchSemanticsNodes()

    /** 跨天那条下面紧挨着的第一条气泡上正好压着一行日期：只有一行，且是核心的分隔条目画的。 */
    private fun oneLineAtTheDayBreak() {
        scrollTo(11)
        compose.waitUntil(10_000) { visible(10) && visible(11) }
        val tenth = bubble(10).fetchSemanticsNode()
        val eleventh = bubble(11).fetchSemanticsNode()
        val between = dateLines().filter {
            it.boundsInRoot.top >= tenth.boundsInRoot.bottom - 1f &&
                it.boundsInRoot.bottom <= eleventh.boundsInRoot.top + 1f
        }
        check(between.size == 1) { "跨天处有 ${between.size} 行日期：${between.map(::texts)}" }
        val line = between.single()
        check(line.config.getOrNull(SemanticsProperties.TestTag) == "day-divider") {
            "跨天处那行不是核心的分隔条目：${texts(line)}"
        }
        check(texts(line) == fixtureDate(11)) { "跨天那行不是第 11 条的日期：${texts(line)}" }
    }

    /** 日期行夹在前一天最后一条和后一天第一条之间：下面紧挨着的那条是它写的这一天，上面那条是更早的一天。 */
    private fun lineIsAboveTheOneItNames(line: SemanticsNode) {
        val mark = line.boundsInRoot
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true)
            .fetchSemanticsNodes().sortedBy { it.boundsInRoot.top }
        check(bubbles.none { it.boundsInRoot.overlaps(mark) }) { "气泡压在日期行上：$mark" }
        val below = bubbles.firstOrNull { it.boundsInRoot.top >= mark.bottom - 1f }
        val belowIndex = checkNotNull(below?.let(::fixtureIndexOf)) { "日期行下面认不出气泡：$mark" }
        check(texts(line) == fixtureDate(belowIndex)) {
            "日期行不是它下面那条的日期：${texts(line)} vs ${fixtureDate(belowIndex)}（第 $belowIndex 条）"
        }
        val above = bubbles.lastOrNull { it.boundsInRoot.bottom <= mark.top + 1f }
        val aboveIndex = above?.let(::fixtureIndexOf) ?: return
        check(fixtureDay(aboveIndex) != fixtureDay(belowIndex)) {
            "同一天（第 $aboveIndex / $belowIndex 条）的气泡之间夹了日期行"
        }
        val order = "日期行上面第 $aboveIndex 条 ${above!!.boundsInRoot}，" +
            "下面第 $belowIndex 条 ${below.boundsInRoot}，日期行 $mark"
        check(fixtureDay(aboveIndex) < fixtureDay(belowIndex)) { "上面的日期不早于下面：$order" }
    }

    /** 同一天相邻的两条之间没有日期行。 */
    private fun noLineBetweenTheSameDay() {
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true)
            .fetchSemanticsNodes().sortedBy { it.boundsInRoot.top }
        val lines = dateLines()
        val sameDay = bubbles.zipWithNext().filter { (above, below) ->
            val a = fixtureIndexOf(above)
            val b = fixtureIndexOf(below)
            a != null && b != null && fixtureDay(a) == fixtureDay(b)
        }
        for ((above, below) in sameDay) {
            val between = lines.filter {
                it.boundsInRoot.top >= above.boundsInRoot.bottom - 1f &&
                    it.boundsInRoot.bottom <= below.boundsInRoot.top + 1f
            }
            val pair = "${fixtureIndexOf(above)} / ${fixtureIndexOf(below)}"
            check(between.isEmpty()) { "同一天的第 $pair 条之间夹了日期行：${between.map(::texts)}" }
        }
    }

    /** 气泡所在消息的序号：按外层条目的标签认，引用的文字里也会出现“Fixture message N”。 */
    private fun fixtureIndexOf(bubble: SemanticsNode): Int? {
        val items = compose.onAllNodes(
            SemanticsMatcher("是消息条目") {
                it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-") == true
            },
            useUnmergedTree = true,
        ).fetchSemanticsNodes()
        val owner = items.firstOrNull { it.boundsInRoot.contains(bubble.boundsInRoot.center) }
        return owner?.config?.getOrNull(SemanticsProperties.TestTag)?.removePrefix("item-ui-fake-")?.toInt()
    }

    /** 节点自己和它下面所有子孙的文字；气泡的标题挂在子节点上。 */
    private fun texts(node: SemanticsNode): String =
        node.config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString() +
            node.children.joinToString("") { texts(it) }

    /** 假数据第 `index` 条的日期：第 0 条早两天，11 起是后一天，其余同一天（同 `ui_fake_room.rs`）。 */
    private fun fixtureDate(index: Int): String =
        DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(fixtureAtMs(index)))

    /** 假数据第 `index` 条所在的哪一天，拿来比先后，免得比字符串。 */
    private fun fixtureDay(index: Int): Date =
        checkNotNull(DateFormat.getDateInstance(DateFormat.MEDIUM).parse(fixtureDate(index))) {
            "假数据的日期解不开：${fixtureDate(index)}"
        }

    private fun fixtureAtMs(index: Int): Long = 1_792_000_000_000L + index * 60_000L +
        if (index >= 11) 86_400_000L else 0L -
        if (index == 0) 172_800_000L else 0L

    /** 日期行里写的哪天。 */
    private fun parseDay(line: SemanticsNode): Date? =
        DateFormat.getDateInstance(DateFormat.MEDIUM).parse(texts(line))

    /** 这条消息的中心在时间线视口里。 */
    private fun visible(index: Int): Boolean {
        val list = compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot
        val center = compose.onAllNodes(hasTestTag("item-ui-fake-$index")).fetchSemanticsNodes().singleOrNull()
            ?.boundsInRoot?.center?.y ?: return false
        return center in list.top..list.bottom
    }

    /** 房间开头那条整个在时间线视口里。 */
    private fun startVisible(): Boolean {
        val list = compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot
        val start = compose.onAllNodes(hasTestTag("item-ui-fake-start"), useUnmergedTree = true)
            .fetchSemanticsNodes().singleOrNull()?.boundsInRoot ?: return false
        return start.top >= list.top - 1f && start.bottom <= list.bottom + 1f
    }

    private fun bubble(index: Int) = compose.onNode(
        hasTestTag("bubble") and hasAnyAncestor(hasTestTag("item-ui-fake-$index")),
        useUnmergedTree = true,
    )

    /** 列表倒着排，按 key 滚过去；条目还没组合出来，`performScrollTo` 找不到。 */
    private fun scrollTo(index: Int) {
        // 时间线是发完再组合的：列表里一条都没有时按 key 滚会报找不到。
        compose.waitUntil(10_000) {
            compose.onAllNodes(SemanticsMatcher("时间线条目") { node ->
                node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-") == true
            }, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-$index")
    }

    private fun pillVisible() = compose.onAllNodes(hasTestTag("date-pill"), useUnmergedTree = true)
        .fetchSemanticsNodes().isNotEmpty()

    private fun screenshot(name: String) {
        // 滚过以后浮动日期胶囊还在，等它淡出再拍，不然截到的是它盖在字上的样子。
        runCatching { compose.waitUntil(5_000) { !pillVisible() } }
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "telegram")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
