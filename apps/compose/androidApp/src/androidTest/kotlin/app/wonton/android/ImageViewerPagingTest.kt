package app.wonton.android

import android.Manifest
import android.accessibilityservice.AccessibilityService
import android.graphics.Bitmap
import android.os.Build
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
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
import org.junit.Assert.assertEquals
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
 * 全屏查看器翻同一次发送的照片（ADR-0094）：左右滑换一张，到头回弹不循环；点保存、分享不翻页。
 * 组里的照片是假房间第 5..7 条（`ui_fake_room.rs` 的 `PHOTO_GROUP_START`），第 3 条是没组的单张。
 */
@RunWith(AndroidJUnit4::class)
class ImageViewerPagingTest {
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
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.onNodeWithText("Echo").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    @Test
    fun swipeTurnsThePageBothWays() {
        openPhoto(1)
        assertEquals("1 / 3", pageText())
        assertEquals("photo-0.bmp", shownPhoto())
        swipeLeft()
        awaitPage("2 / 3")
        assertEquals("photo-1.bmp", shownPhoto())
        swipeRight()
        awaitPage("1 / 3")
        assertEquals("photo-0.bmp", shownPhoto())
        screenshot("swipe-both-ways")
    }

    @Test
    fun swipePastTheFirstPhotoBouncesBack() {
        openPhoto(1)
        swipeRight()
        awaitSettled()
        assertEquals("1 / 3", pageText())
        assertEquals("photo-0.bmp", shownPhoto())
        screenshot("first-photo-bounce")
        // 到末尾再往左拉也一样。
        swipeLeft()
        awaitPage("2 / 3")
        swipeLeft()
        awaitPage("3 / 3")
        swipeLeft()
        awaitSettled()
        assertEquals("3 / 3", pageText())
        assertEquals("photo-2.bmp", shownPhoto())
        screenshot("last-photo-bounce")
    }

    @Test
    fun tappingTheSidesTurnsThePage() {
        openPhoto(1)
        tapSide(0.9f)
        awaitPage("2 / 3")
        tapSide(0.1f)
        awaitPage("1 / 3")
    }

    @Test
    fun saveAndShareDoNotTurnThePage() {
        openPhoto(2)
        assertEquals("2 / 3", pageText())
        compose.onNodeWithContentDescription(string(R.string.message_image_save)).performClick()
        compose.waitForIdle()
        assertEquals("2 / 3", pageText())
        compose.onNodeWithContentDescription(string(R.string.message_image_share)).performClick()
        // 分享面板是系统的另一个窗口，盖住 App 后 Compose 就找不到本 App 的层级：等它盖上来再关。
        compose.waitUntil(10_000) { covered() }
        instrumentation.uiAutomation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
        compose.waitUntil(10_000) { !covered() && viewerPageShown() }
        compose.onNodeWithTag("viewer").assertIsDisplayed()
        assertEquals("2 / 3", pageText())
        screenshot("save-share")
    }

    @Test
    fun singlePhotoHasNoPageCounterAndDoesNotMove() {
        openItem(3)
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("viewer")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("viewer-page").assertDoesNotExist()
        swipeLeft()
        awaitSettled()
        assertEquals("photo-3.bmp", shownPhoto())
        screenshot("single-photo")
    }

    /** 第 `position` 张组照片（1 起）连同页码一起等出来。 */
    private fun openPhoto(position: Int) {
        openItem(position + 4)
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("viewer-page")).fetchSemanticsNodes().isNotEmpty() }
    }

    /** 滚到第 `index` 条并点开它：图片充满整条气泡，点条目中间就是点图。 */
    private fun openItem(index: Int) {
        compose.waitUntil(10_000) {
            compose.onAllNodes(SemanticsMatcher("时间线条目") { node ->
                node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-") == true
            }).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-$index")
        compose.onNodeWithTag("item-ui-fake-$index", useUnmergedTree = true).performClick()
    }

    /** 页码变成期望值：手势之后落定有动画，等它停。 */
    private fun awaitPage(expected: String) {
        compose.waitUntil(5_000) { pageText() == expected }
    }

    /** 页码没变的手势也要等它弹回去，不然断言可能抢在动画前。 */
    private fun awaitSettled() {
        compose.mainClock.advanceTimeBy(1_000)
        compose.waitForIdle()
    }

    private fun pageText(): String = compose.onNodeWithTag("viewer-page").fetchSemanticsNode()
        .config[SemanticsProperties.Text].joinToString()

    /** 本 App 的层级找不到了：多半是系统窗口（分享面板）盖在上面。 */
    private fun covered(): Boolean = runCatching {
        compose.onAllNodes(hasTestTag("viewer-page")).fetchSemanticsNodes()
    }.isFailure

    private fun viewerPageShown(): Boolean = runCatching {
        compose.onAllNodes(hasTestTag("viewer-page")).fetchSemanticsNodes().isNotEmpty()
    }.getOrDefault(false)

    private fun shownPhoto(): String? = compose.onNodeWithTag("viewer-image", useUnmergedTree = true)
        .fetchSemanticsNode().config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString()

    private fun swipeLeft() = swipe(from = 0.8f, to = 0.2f)

    private fun swipeRight() = swipe(from = 0.2f, to = 0.8f)

    private fun swipe(from: Float, to: Float) {
        compose.onNodeWithTag("viewer").performTouchInput {
            val y = height / 2f
            swipe(Offset(width * from, y), Offset(width * to, y), durationMillis = 200)
        }
    }

    private fun tapSide(fraction: Float) {
        compose.onNodeWithTag("viewer").performTouchInput { click(Offset(width * fraction, height / 2f)) }
    }

    private fun string(id: Int) = compose.activity.getString(id)

    private fun screenshot(name: String) {
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "viewer")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
