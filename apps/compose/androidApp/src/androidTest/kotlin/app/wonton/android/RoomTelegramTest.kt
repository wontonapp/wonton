package app.wonton.android

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import android.view.WindowInsets
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.R
import app.wonton.ui.send
import java.io.File
import java.io.FileOutputStream
import kotlin.math.abs
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith

/**
 * Telegram 样式的聊天页（ADR-0109）：左滑回复、点引用跳转和“向下”回去、命令菜单、选项按钮。
 * 只用 Debug 的假核心和它固定的房间，不连家里的服务器。
 */
@RunWith(AndroidJUnit4::class)
class RoomTelegramTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
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
        compose.onNodeWithText("Echo").performClick()
        // 假房间的落点在第三条，先把输入框等出来，各用例自己滚到要的那一条。
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

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
        compose.waitUntil(10_000) { visible(5) && !visible(21) }
        screenshot("jump-highlight")
        // 离开底部时“向下”先退回跳之前的那条，不直接回最新。
        compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom)).performClick()
        compose.waitUntil(10_000) { visible(21) }
        screenshot("jump-back")
    }

    @Test
    fun commandMenuListsBotCommandsAndSends() {
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.onNodeWithTag("command-help").assertIsDisplayed()
        compose.onNodeWithTag("command-model").assertIsDisplayed()
        settleKeyboard()
        screenshot("command-menu")
        compose.onNodeWithTag("command-new").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("/new", substring = true, useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        settleKeyboard()
        screenshot("command-sent")
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
            hasTestTag("choices-row"),
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
        // 23 是最早的一条，翻到最上面就是它。
        scrollTo(23)
        repeat(3) {
            compose.onNodeWithTag("room-timeline").performTouchInput {
                swipe(Offset(centerX, centerY * 0.3f), Offset(centerX, centerY * 1.9f), durationMillis = 400)
            }
            compose.waitForIdle()
        }
        bubble(23).assertIsDisplayed()
        // 最早那条气泡的顶边还在顶栏下面，没被顶栏拿走。
        val first = bubble(23).fetchSemanticsNode().boundsInRoot
        // 顶栏和它下面那行（群里的 AI 等）都不能压在第一条上。
        val bar = compose.onNodeWithTag("room-bar").fetchSemanticsNode().boundsInRoot.bottom
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot.bottom
        val limit = maxOf(bar, header)
        check(first.top >= limit) { "第一条被顶栏盖住：${first.top} < $limit" }
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
    fun daySeparatorOwnsARowBetweenDays() {
        // 第 11 条比上一条早一天：跨日的那条上方是一行固定的日期，它自己占一行。
        // 分隔行贴着跨日的那两条，翻到附近才进屏幕；两边各翻几条，撞上为止。
        var separators = emptyList<SemanticsNode>()
        var seen = ""
        for (index in listOf(10, 9, 11, 8, 12, 13)) {
            scrollTo(index)
            compose.waitForIdle()
            separators = compose.onAllNodes(hasTestTag("day-separator"), useUnmergedTree = true)
                .fetchSemanticsNodes()
            seen += "$index:${separators.size} "
            if (separators.isNotEmpty()) break
        }
        check(separators.isNotEmpty()) { "没画日期分隔：$seen" }
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true).fetchSemanticsNodes()
        for (separator in separators) {
            val mark = separator.boundsInRoot
            check(bubbles.none { it.boundsInRoot.overlaps(mark) }) { "气泡压在日期分隔上：$mark" }
            check(bubbles.any { it.boundsInRoot.bottom <= mark.top + 1f }) { "日期分隔上面没有更早的气泡：$mark" }
            check(bubbles.any { it.boundsInRoot.top >= mark.bottom - 1f }) { "日期分隔下面没有更新的气泡：$mark" }
        }
    }

    /** 回到房间列表再进群；用例都在同一个活动里连着跑。 */
    private fun openFamily() {
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithText("Family").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    private fun avatars() =
        compose.onAllNodes(hasTestTag("sender-avatar"), useUnmergedTree = true).fetchSemanticsNodes()

    /** 键盘动画比布局慢一拍，等表面画上来再截，不然截到的还是刚才那一帧。 */
    private fun settleKeyboard() {
        compose.waitUntil(10_000) { imeHeight() > 0 }
        compose.waitForIdle()
        Thread.sleep(300)
    }

    private fun pillVisible() = compose.onAllNodes(hasTestTag("date-pill"), useUnmergedTree = true)
        .fetchSemanticsNodes().isNotEmpty()

    @Test
    fun commandMenuKeepsBarAndComposerInPlace() {
        val statusBar = statusBarHeight()
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isNotEmpty() }
        settleKeyboard()
        // 键盘起来以后顶栏原地不动，输入框压在键盘上沿，时间线还在。
        // 顶栏的盒子从窗口顶起，看它里面的标题：标题还在状态栏下面就是没被顶走。
        val title = compose.onAllNodesWithText("Echo", useUnmergedTree = true)
            .fetchSemanticsNodes().first().boundsInRoot
        check(title.top >= statusBar - 1f) { "顶栏被顶到状态栏底下：${title.top} < $statusBar" }
        val bar = compose.onNodeWithTag("room-bar").fetchSemanticsNode().boundsInRoot
        check(bar.bottom >= statusBar) { "顶栏整条被顶走：${bar.bottom} < $statusBar" }
        val input = compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().single().boundsInRoot
        val keyboardTop = compose.activity.window.decorView.height - imeHeight()
        // 输入区整条贴着键盘上沿，输入框在它最下沿（自己留了一点内边距）。
        val block = compose.onNodeWithTag("composer-block").fetchSemanticsNode().boundsInRoot
        val gap = keyboardTop - block.bottom
        check(abs(gap) <= 2f) { "输入区没贴着键盘上沿：差 $gap" }
        check(input.bottom <= block.bottom + 0.5f) { "输入框掉出输入区：${input.bottom} > ${block.bottom}" }
        // 命令多的时候输入区顶到顶栏下沿为止，顶栏本身不能被它盖住。
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot
        check(block.top >= header.bottom - 1f) { "输入区盖住顶栏：${block.top} < ${header.bottom}" }
        check(compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot.height > 0f)
        // 菜单盖住输入框上方，“向下”钮和菜单叠在状态栏边，菜单开着时不画它。
        compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom)).assertIsNotDisplayed()
        screenshot("command-menu")
    }

    @Test
    fun pluginStatusRowCoversNoBubble() {
        // 群里 Bot 掉线的那行（假房间里是 Hermes）自己占一条，时间线让开它，气泡不压在它上面。
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

    /** 这条消息的中心在时间线视口里。跳转过后 `isDisplayed` 会因为列表项没 placed 而恒假。 */
    private fun visible(index: Int): Boolean {
        val list = compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot
        val center = compose.onAllNodes(hasTestTag("item-ui-fake-$index")).fetchSemanticsNodes().singleOrNull()
            ?.boundsInRoot?.center?.y ?: return false
        return center in list.top..list.bottom
    }

    private fun item(index: Int) = compose.onNodeWithTag("item-ui-fake-$index")

    private fun bubble(index: Int) = compose.onNode(
        hasTestTag("bubble") and hasAnyAncestor(hasTestTag("item-ui-fake-$index")),
        useUnmergedTree = true,
    )

    /** 列表倒着排，按 key 滚过去；条目还没组合出来，`performScrollTo` 找不到。 */
    private fun scrollTo(index: Int) = compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-$index")

    /** 有 `Disabled` 语义就不算能用。 */
    private fun SemanticsNodeInteraction.isEnabled(): Boolean =
        SemanticsProperties.Disabled !in fetchSemanticsNode().config

    /** 选项的选中和停用语义在内层那行，标签在外层，按祖先找；选中与否两种状态都有 `Selected`。 */
    private fun choice(key: String) = compose.onNode(
        hasAnyAncestor(hasTestTag("choice-$key")) and
            SemanticsMatcher.keyIsDefined(SemanticsProperties.Selected),
        useUnmergedTree = true,
    )

    private fun string(id: Int) = compose.activity.getString(id)

    /** 状态栏和键盘的高度（像素），从窗口的实时 insets 取。 */
    private fun statusBarHeight(): Int =
        compose.activity.window.decorView.rootWindowInsets.getInsets(WindowInsets.Type.statusBars()).top

    private fun imeHeight(): Int =
        compose.activity.window.decorView.rootWindowInsets.getInsets(WindowInsets.Type.ime()).bottom

    private fun screenshot(name: String) {
        // 滚过以后浮动日期胶囊还在，等它淡出再拍，不然截到的是它盖在字上的样子；失败现场可能已经停了，等不到也照拍。
        runCatching { compose.waitUntil(5_000) { !pillVisible() } }
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "telegram")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
