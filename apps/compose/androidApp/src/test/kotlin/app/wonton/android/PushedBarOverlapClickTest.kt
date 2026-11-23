@file:Suppress("MagicNumber") // 行高和让位的 dp 是测试自定的几何。

package app.wonton.android

import android.app.Application
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.isHeading
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.ui.R
import app.wonton.ui.design.LocalWontonTopInset
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.Home
import app.wonton.ui.icons.WontonIcon
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
private const val LONG_TITLE = "一个长到需要省略而且还要再长一些的标题，用来确认夹紧"

/**
 * 推进栏（[WontonPushedBar]）盖住的内容照样点得到：内容滚进半透明顶栏下面以后，点在它身上要触发它自己，
 * 不能被栏子树拦下（#688：特大字号时明暗选项滚进栏下，点“深色”没有反应）。
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
class PushedBarOverlapClickTest {
    @get:Rule
    val compose = createComposeRule()

    private val context = ApplicationProvider.getApplicationContext<Application>()

    @Test
    fun aRowScrolledUnderThePushedBarStillTakesTheClick() = assertOverlappedRowTakesTheClick(centered = false)

    @Test
    fun aRowUnderTheCenteredTitleStillTakesTheClick() = assertOverlappedRowTakesTheClick(centered = true)

    private fun assertOverlappedRowTakesTheClick(centered: Boolean) {
        var clicks by mutableIntStateOf(0)
        val scroll = ScrollState(0)
        compose.setContent {
            WontonTheme(testAppearance()) {
                WontonScreen(
                    bar = {
                        Box(Modifier.testTag("bar")) {
                            WontonPushedBar(title = "一个长到要省略的标题", onBack = {}, centeredTitle = centered)
                        }
                    },
                ) {
                    Column(
                        // 和真实页面同序（AppearanceScreen.kt:78-80）：让位是滚动内容自己的内边距，
                        // 滚起来的内容才会从栏下滑过去；放在 verticalScroll 外面就被栏沿裁掉了。
                        Modifier.fillMaxSize()
                            .verticalScroll(scroll)
                            .wontonContentPadding()
                            .testTag("content"),
                    ) {
                        Spacer(Modifier.height(200.dp))
                        Box(
                            Modifier.fillMaxWidth()
                                .height(96.dp)
                                .clickable { clicks++ }
                                .testTag("row"),
                            contentAlignment = Alignment.Center,
                        ) {
                            BasicText("深色")
                        }
                        Spacer(Modifier.height(2000.dp))
                    }
                }
            }
        }
        compose.waitForIdle()

        // 对照：行在栏下方时点得中，证明点击注入本身是通的。
        compose.onNodeWithTag("row").performClick()
        compose.runOnIdle { assertEquals(1, clicks) }

        // 把行滚到栏的正中，再点它身上：这一刻它整体盖在栏下，和 #688 的场景一样。
        val bar = compose.onNodeWithTag("bar").getUnclippedBoundsInRoot()
        val center = with(compose.density) {
            Offset(((bar.left + bar.right) / 2).toPx(), ((bar.top + bar.bottom) / 2).toPx())
        }
        val halfBar = with(compose.density) { (bar.bottom - bar.top).toPx() } / 2
        val shift = compose.onNodeWithTag("row").fetchSemanticsNode().boundsInRoot.center.y - center.y
        assertTrue("行中心得在栏中心下方才滚得动", shift > 0f)
        compose.runOnIdle { scroll.dispatchRawDelta(shift) }
        compose.waitForIdle()

        val row = compose.onNodeWithTag("row").fetchSemanticsNode().boundsInRoot
        val point = row.center
        assertTrue("点击点 ($point) 要落在栏 ($bar) 里，几何变了测试就测不到这条", point.y in (center.y - halfBar)..(center.y + halfBar))
        assertTrue("点击点得在行里", row.contains(point))

        compose.onRoot().performTouchInput { down(point); up() }
        compose.runOnIdle { assertEquals("栏下的行没收到点击", 2, clicks) }
    }

    /** 栏自己的东西还在：标题是读屏的标题，返回是 48dp 的可点按钮，栏高保持 M3 的 64dp。 */
    @Test
    fun theBarKeepsItsHeadingBackButtonAndHeight() {
        var backs = 0
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.testTag("bar")) {
                    WontonPushedBar(title = "外观", onBack = { backs++ })
                }
            }
        }
        compose.onNodeWithText("外观").assert(isHeading())
        val bar = compose.onNodeWithTag("bar").getUnclippedBoundsInRoot()
        assertEquals("栏高要和 M3 的单行栏一致", 64.0, (bar.bottom - bar.top).value.toDouble(), 0.5)
        val back = compose.onNodeWithContentDescription(context.getString(R.string.design_back))
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Button))
            .getUnclippedBoundsInRoot()
        assertEquals("返回键的命中区是 48dp", 48.0, (back.right - back.left).value.toDouble(), 0.5)
        assertEquals(48.0, (back.bottom - back.top).value.toDouble(), 0.5)
        compose.onNodeWithContentDescription(context.getString(R.string.design_back)).performClick()
        compose.runOnIdle { assertEquals(1, backs) }
    }

    @Test
    fun centeredTitleSitsOnTheBarCenterWithActions() {
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.testTag("bar")) {
                    WontonPushedBar(
                        title = "外观", onBack = {}, centeredTitle = true,
                        actions = { WontonIconButton(WontonIcon.Home, "home", {}) },
                    )
                }
            }
        }
        assertTitleIsCenteredOnTheBar()
    }

    @Test
    fun centeredTitleSitsOnTheBarCenterWithoutActions() {
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.testTag("bar")) {
                    WontonPushedBar(title = "外观", onBack = {}, centeredTitle = true)
                }
            }
        }
        assertTitleIsCenteredOnTheBar()
    }

    /** 标题中心要对准栏中心（M3 的夹紧只在压到两边时才生效），且整条在栏内。 */
    private fun assertTitleIsCenteredOnTheBar() {
        val bar = compose.onNodeWithTag("bar").getUnclippedBoundsInRoot()
        val title = compose.onNodeWithText("外观").getUnclippedBoundsInRoot()
        assertEquals(
            ((bar.left + bar.right).value / 2).toDouble(),
            ((title.left + title.right).value / 2).toDouble(),
            0.5,
        )
        assertTrue("标题要整条在栏内", title.left >= bar.left && title.right <= bar.right)
    }

    @Test
    fun aLongCenteredTitleStopsAtTheBackButton() {
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.testTag("bar")) {
                    WontonPushedBar(
                        title = LONG_TITLE, onBack = {}, centeredTitle = true,
                        actions = { WontonIconButton(WontonIcon.Home, "home", {}) },
                    )
                }
            }
        }
        compose.waitForIdle()
        val back = compose.onNodeWithContentDescription(context.getString(R.string.design_back))
            .getUnclippedBoundsInRoot()
        val title = compose.onNodeWithText(LONG_TITLE).getUnclippedBoundsInRoot()
        assertTrue("长标题不能压到返回键底下", title.left >= back.right)
    }

    @Test
    fun aLongLeftTitleStopsBeforeTheActions() {
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.testTag("bar")) {
                    WontonPushedBar(
                        title = LONG_TITLE, onBack = {},
                        actions = { WontonIconButton(WontonIcon.Home, "home", {}) },
                    )
                }
            }
        }
        compose.waitForIdle()
        val back = compose.onNodeWithContentDescription(context.getString(R.string.design_back))
            .getUnclippedBoundsInRoot()
        val action = compose.onNodeWithContentDescription("home").getUnclippedBoundsInRoot()
        val title = compose.onNodeWithText(LONG_TITLE).getUnclippedBoundsInRoot()
        assertTrue("长标题要从返回键右边起", title.left >= back.right)
        assertTrue("长标题要在 actions 左边收住", title.right <= action.left)
    }

    /**
     * 内容第一遍组合就拿得到栏高，内容让位等于栏底。
     *
     * 宿主最外层先给有限尺寸：不定尺寸时宿主首测是 0 约束，栏也量成 0，第一遍就测不出差别。
     * 旧实现（栏高靠 `onSizeChanged` 回填）第一遍固定是 0；`waitForIdle` 之后它也会收敛到 64，所以只断言稳定值锁不住它。
     */
    @Test
    fun contentSeesTheBarHeightOnItsFirstComposition() {
        var firstInset: Dp? = null
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.size(320.dp, 640.dp)) {
                    WontonScreen(
                        bar = { Box(Modifier.testTag("bar")) { WontonPushedBar(title = "外观", onBack = {}) } },
                    ) {
                        if (firstInset == null) firstInset = LocalWontonTopInset.current
                        Column(Modifier.wontonContentPadding()) { BasicText("内容") }
                    }
                }
            }
        }
        compose.waitForIdle()
        val bar = compose.onNodeWithTag("bar").getUnclippedBoundsInRoot()
        val content = compose.onNodeWithText("内容").getUnclippedBoundsInRoot()
        val barHeight = (bar.bottom - bar.top).value.toDouble()
        assertEquals("栏高是 M3 的 64dp", 64.0, barHeight, 0.5)
        assertEquals("内容第一遍组合就要拿到栏高", barHeight, firstInset?.value?.toDouble() ?: -1.0, 0.5)
        assertEquals("内容让位要从栏底开始", bar.bottom.value.toDouble(), content.top.value.toDouble(), 0.5)
    }
}
