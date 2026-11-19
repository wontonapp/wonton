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
        screenshot("command-menu")
        compose.onNodeWithTag("command-new").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("/new", substring = true, useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
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

    /** 回到房间列表再进群；用例都在同一个活动里连着跑。 */
    private fun openFamily() {
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithText("Family").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    private fun avatars() =
        compose.onAllNodes(hasTestTag("sender-avatar"), useUnmergedTree = true).fetchSemanticsNodes()

    private fun pillVisible() = compose.onAllNodes(hasTestTag("date-pill"), useUnmergedTree = true)
        .fetchSemanticsNodes().isNotEmpty()

    @Test
    fun commandMenuKeepsBarAndComposerInPlace() {
        val statusBar = statusBarHeight()
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isNotEmpty() }
        compose.waitUntil(10_000) { imeHeight() > 0 }
        screenshot("command-menu")
        // 键盘起来以后顶栏原地不动，输入框压在键盘上沿，时间线还在。
        // 顶栏的盒子从窗口顶起，看它里面的标题：标题还在状态栏下面就是没被顶走。
        val title = compose.onAllNodesWithText("Echo", useUnmergedTree = true)
            .fetchSemanticsNodes().first().boundsInRoot
        check(title.top >= statusBar - 1f) { "顶栏被顶到状态栏底下：${title.top} < $statusBar" }
        val bar = compose.onNodeWithTag("room-bar").fetchSemanticsNode().boundsInRoot
        check(bar.bottom >= statusBar) { "顶栏整条被顶走：${bar.bottom} < $statusBar" }
        val input = compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().single().boundsInRoot
        val keyboardTop = compose.activity.window.decorView.height - imeHeight()
        check(input.bottom <= keyboardTop + 1f) { "输入框没压在键盘上沿：${input.bottom} > $keyboardTop" }
        check(input.bottom >= keyboardTop - 40f) { "输入框被顶离键盘：${input.bottom} < $keyboardTop" }
        check(compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot.height > 0f)
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
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "telegram")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
