package app.wonton.android

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import android.os.SystemClock
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.semantics.SemanticsNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.protocol.PageAction
import app.wonton.protocol.SettingsAction
import app.wonton.protocol.TabRef
import app.wonton.protocol.ThemeMode
import app.wonton.ui.R
import app.wonton.ui.send
import java.io.File
import java.io.FileOutputStream
import java.text.DateFormat
import java.util.Date
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import kotlinx.coroutines.runBlocking
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith

/** 引用一条不在房间里的消息（`ui_fake_room.rs` 的 `MISSING_QUOTE_INDEX`）。 */
private const val MISSING_QUOTE = 24
/** 删除的消息，菜单里没有动作（`ui_fake_room.rs` 的 `REDACTED_INDEX`）。 */
private const val REDACTED = 25
/** 名字很长的发送者（`ui_fake_room.rs` 的 `LONG_NAME_INDEX`）。 */
private const val LONG_NAME = 26

private const val ECHO_ROOM = "!ui-fake:home.example"
/** 引用第 5 条的那条（`ui_fake_room.rs` 的 `REPLY_INDEX`）。 */
private const val REPLY = 21

/** 引用跳转和长按菜单的修复：引用可点、找不到的提示会消失、同一条再点还会回去、菜单写发送时间。 */
@RunWith(AndroidJUnit4::class)
class RoomQuoteFixTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.READ_MEDIA_IMAGES)
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
        // 别的类跑过以后附件区可能还留着上传失败的图，发送（含点命令）会被拒；本类不测附件，先清空。
        runBlocking {
            core.subscribeComposer(TimelineTarget(ECHO_ROOM, null)).use { sub ->
                sub.next()!!.attachments.forEach { item ->
                    core.send(
                        Action.Timeline(
                            TimelineAction.RemoveAttachment(TimelineTarget(ECHO_ROOM, null), item.id),
                        ),
                    )
                }
            }
        }
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    @Test
    fun longSenderNameStaysOneLine() {
        openFamily()
        scrollTo(LONG_NAME)
        val name = compose.onNode(
            hasTestTag("senderName") and
                hasText(LONG_NAME_TEXT) and
                hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_NAME")) and
                SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult),
            useUnmergedTree = true,
        )
        name.assertExists()
        val layout = layoutOf(name.fetchSemanticsNode())
        checkNotNull(layout)
        check(layout.lineCount == 1) { "发送者名字占了 ${layout.lineCount} 行" }
        val nameWidth = name.fetchSemanticsNode().boundsInRoot.width
        val bubbleWidth = bubble(LONG_NAME).fetchSemanticsNode().boundsInRoot.width
        check(nameWidth <= bubbleWidth) { "名字比气泡宽：$nameWidth > $bubbleWidth" }
    }

    @Test
    fun menuEndsWithSentTime() {
        scrollTo(11)
        bubble(11).performTouchInput { longClick() }
        val expected = sentAt(11)
        compose.onNodeWithText(expected).assertIsDisplayed()
    }

    @Test
    fun redactedMessageStillOpensMenuWithSentTime() {
        scrollTo(REDACTED)
        bubble(REDACTED).performTouchInput { longClick() }
        compose.onNodeWithText(sentAt(REDACTED)).assertIsDisplayed()
        // 没有可复制的正文，菜单里不该冒出复制。
        compose.onNodeWithText(string(R.string.message_copy)).assertDoesNotExist()
    }

    @Test
    fun readOutFindsReceiptAndEditMark() {
        scrollTo(23)
        compose.onNode(
            hasContentDescription(string(R.string.message_delivered)) and hasAnyAncestor(hasTestTag("item-ui-fake-23")),
            useUnmergedTree = true,
        ).assertExists()

        scrollTo(REDACTED)
        compose.onNode(
            hasTestTag("messageEdited") and hasAnyAncestor(hasTestTag("item-ui-fake-$REDACTED")),
            useUnmergedTree = true,
        ).assertTextEquals(string(R.string.timeline_edited))
    }

    @Test
    fun missingQuoteShowsPillAboveComposerThenHides() {
        val pill = pill()
        tapQuote(MISSING_QUOTE)
        compose.waitUntil(10_000) { displayed(pill) }
        val pillBox = pill.fetchSemanticsNode().boundsInRoot
        val composer = compose.onNodeWithTag("composer-block").fetchSemanticsNode().boundsInRoot
        check(pillBox.bottom <= composer.top) { "提示压着输入框：${pillBox.bottom} > ${composer.top}" }
        // 提示最多停两秒，自己消失；等不到就是没消失。
        compose.waitUntil(10_000) { !displayed(pill) }
    }

    @Test
    fun missingQuoteCanBeTappedAgain() {
        val pill = pill()
        tapQuote(MISSING_QUOTE)
        compose.waitUntil(10_000) { displayed(pill) }
        // 提示还在的时候再点一次也要打到底下的引用，不能被提示吃掉这次点击。
        tapQuote(MISSING_QUOTE)
        compose.waitUntil(10_000) { !displayed(pill) }
        compose.waitUntil(10_000) { displayed(pill) }
    }

    @Test
    fun sendingACommandKeepsTheDraftAndClosesTheReply() {
        scrollTo(REPLY)
        compose.waitForIdle()
        item(REPLY).performTouchInput {
            swipe(Offset(width * 0.8f, centerY), Offset(width * 0.3f, centerY), durationMillis = 300)
        }
        compose.waitUntil(10_000) { displayed(compose.onNodeWithTag("composer-quote")) }
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.onNode(hasTestTag("command-help"), useUnmergedTree = true).performClick()
        // 命令发出去后草稿留着，回复条收掉（ADR-0109 第 4 节）。
        compose.waitUntil(10_000) { !displayed(compose.onNodeWithTag("composer-quote")) }
        compose.onNode(hasSetTextAction()).assertTextEquals("/help")
        compose.onNodeWithTag("composer-quote").assertDoesNotExist()
    }

    @Test
    fun quoteShowsSpinnerAndRefusesRepeatedTapsWhileJumping() {
        tapQuote(MISSING_QUOTE)
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("quote-loading", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        // 转圈的时候再点不派第二次跳转：假核心一次跳转 700ms 后放弃，派了第二次就会拖到 1400ms 才收。
        quote(MISSING_QUOTE).performClick()
        val started = SystemClock.elapsedRealtime()
        compose.waitUntil(2_000) {
            compose.onAllNodesWithTag("quote-loading", useUnmergedTree = true).fetchSemanticsNodes().isEmpty()
        }
        val took = SystemClock.elapsedRealtime() - started
        assertTrue("第二下又派了一次跳转，转圈 $took ms 才收", took < 1_200)
    }

    @Test
    fun replyBarJumpsBackToTheOriginal() {
        scrollTo(REPLY)
        item(REPLY).performTouchInput {
            swipe(Offset(width * 0.8f, centerY), Offset(width * 0.3f, centerY), durationMillis = 300)
        }
        scrollTo(0)
        compose.waitForIdle()
        check(!visible(REPLY)) { "还没滚动就看得见第 21 条" }
        // 回复条引的是被回的那条（21 自己），点它跳回去。
        compose.onNode(hasTestTag("composer-quote") and hasText(QUOTE_SUMMARY, substring = true))
            .performClick()
        compose.waitUntil(10_000) { visible(REPLY) }
    }

    @Test
    fun tappingTheSameQuoteTwiceRelocatesBothTimes() {
        scrollTo(REPLY)
        quote(REPLY).performClick()
        compose.waitUntil(10_000) { visible(5) }
        scrollTo(REPLY)
        compose.waitForIdle()
        check(!visible(5)) { "第二次点之前第 5 条还在屏幕上" }
        quote(REPLY).performClick()
        compose.waitUntil(10_000) { visible(5) }
    }

    @Test
    fun timelineFixesScreenshots() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        val original = core.appearance().themeMode
        try {
            for (mode in listOf(ThemeMode.LIGHT, ThemeMode.DARK)) {
                assertNull(core.send(Action.Settings(SettingsAction.SetThemeMode(mode))))
                compose.waitForIdle()
                val prefix = mode.name.lowercase()
                openFamily()
                awaitTheme(mode)
                scrollTo(LONG_NAME)
                screenshot("$prefix-long-name")
                compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
                compose.onNodeWithText("Echo").performClick()
                compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
                scrollTo(11)
                bubble(11).performTouchInput { longClick() }
                screenshot("$prefix-menu-sent-at")
                // 菜单底下的时间是普通文字，点它不关菜单；按返回关掉（弹窗自己收）。
                compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
                compose.waitUntil(5_000) {
                    compose.onAllNodesWithTag("message-sent-at", useUnmergedTree = true).fetchSemanticsNodes().isEmpty()
                }
                val pill = pill()
                tapQuote(MISSING_QUOTE)
                compose.waitUntil(10_000) { displayed(pill) }
                screenshot("$prefix-missing-quote", waitForPill = false)
            }
        } finally {
            assertNull(core.send(Action.Settings(SettingsAction.SetThemeMode(original))))
        }
    }

    private fun openFamily() {
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithText("Family").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    /** 第 5 条的中心在时间线视口里。 */
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

    private fun quote(index: Int) = compose.onNode(
        hasTestTag("quote") and hasAnyAncestor(hasTestTag("item-ui-fake-$index")),
        useUnmergedTree = true,
    )

    /** 引用行在很长的气泡顶上，先把它滚进屏幕再点，点才落得到。 */
    private fun tapQuote(index: Int) {
        scrollTo(index)
        quote(index).performScrollTo()
        quote(index).performClick()
    }

    private fun scrollTo(index: Int) {
        compose.waitUntil(10_000) {
            compose.onAllNodes(SemanticsMatcher("时间线条目") { node ->
                node.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag)
                    ?.startsWith("item-ui-fake-") == true
            }, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-$index")
    }

    private fun layoutOf(node: SemanticsNode): TextLayoutResult? {
        val action = node.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action ?: return null
        val results = mutableListOf<TextLayoutResult>()
        return if (action.invoke(results)) results.firstOrNull() else null
    }

    private fun pill() = compose.onNode(
        hasTestTag("jump-hint") and hasText(string(R.string.timeline_jump_unavailable)),
        useUnmergedTree = true,
    )

    private fun displayed(node: SemanticsNodeInteraction): Boolean =
        runCatching { node.fetchSemanticsNode() }.isSuccess

    private fun string(id: Int, vararg args: Any) = compose.activity.getString(id, *args)

    /** 菜单里写的发送时间：第 11 条起假数据整体晚一天。 */
    private fun sentAt(index: Int): String = string(
        R.string.message_sent_at,
        DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
            .format(Date(AT_MS + index * 60_000L + if (index >= 11) 86_400_000L else 0)),
    )

    /** 外观经后台订阅回流，界面不是发完就换色；按画布像素等到真换上再拍。 */
    private fun awaitTheme(mode: ThemeMode) {
        val dark = mode == ThemeMode.DARK
        compose.waitUntil(10_000) {
            val image = instrumentation.uiAutomation.takeScreenshot()
            val small = android.graphics.Bitmap.createScaledBitmap(image, 8, 16, false)
            image.recycle()
            var luma = 0.0
            for (x in 0 until 8) for (y in 0 until 16) {
                val pixel = small.getPixel(x, y)
                luma += 0.299 * android.graphics.Color.red(pixel) + 0.587 * android.graphics.Color.green(pixel) +
                    0.114 * android.graphics.Color.blue(pixel)
            }
            small.recycle()
            val average = luma / (8 * 16 * 255)
            if (dark) average < 0.5 else average > 0.5
        }
    }

    private fun screenshot(name: String, waitForPill: Boolean = true) {
        if (waitForPill) {
            runCatching {
                compose.waitUntil(5_000) {
                    compose.onAllNodesWithTag("date-pill", useUnmergedTree = true).fetchSemanticsNodes().isEmpty()
                }
            }
        }
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "quote-fix")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }

}

/** 第 26 条的发送者名字，和假数据一致。 */
private const val LONG_NAME_TEXT = "Echo the group bot with a really long account name"
/** 第 21 条引用内容的原文。 */
private const val QUOTE_SUMMARY = "Fixture message $REPLY"
/** 假数据的起始时间；第 11 条起假数据整体晚一天。 */
private const val AT_MS = 1_792_000_000_000L
