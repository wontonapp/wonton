package app.wonton.android

import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import app.wonton.ui.R
import kotlin.math.abs
import org.junit.Test

/** 命令菜单（ADR-0109 第 7 节）：敲“/”出列表、按名字过滤、点命令保住草稿、返回和翻时间线只收列表。 */
class RoomCommandMenuTest : RoomTelegramCase() {
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

    /** 只打一个“/”就出整张列表；返回只收列表，不退出房间。 */
    @Test
    fun slashAloneOpensTheList() {
        compose.onNode(hasSetTextAction()).performTextInput("/")
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isNotEmpty() }
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.waitForIdle()
        check(compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isEmpty()) { "返回键没收起命令列表" }
        check(
            compose.onAllNodes(hasText(ECHO_NAME) and hasAnyAncestor(hasTestTag("room-bar")))
                .fetchSemanticsNodes().isNotEmpty(),
        ) { "返回键把房间关了" }
    }

    @Test
    fun commandMenuKeepsDraftAndBackOnlyClosesList() {
        val field = compose.onNode(hasSetTextAction())
        field.performTextInput("先说一句")
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isNotEmpty() }
        field.assert(hasText("先说一句"))
        // 点一条命令：照常发出去，打了一半的字还在。
        compose.onNodeWithTag("command-new").performClick()
        compose.waitForIdle()
        field.assert(hasText("先说一句"))
        check(compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isEmpty()) { "发完命令列表还开着" }
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("/new", substring = true, useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        // 再开列表，按返回只收列表，人还在房间里。
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isNotEmpty() }
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.waitForIdle()
        check(compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isEmpty()) { "返回键没收起命令列表" }
        check(
            compose.onAllNodes(hasText(ECHO_NAME) and hasAnyAncestor(hasTestTag("room-bar")))
                .fetchSemanticsNodes().isNotEmpty(),
        ) { "返回键把房间关了" }
    }

    /** 翻时间线时列表收起，把列表让出来（和 iOS 一样）。 */
    @Test
    fun scrollingTheTimelineClosesTheList() {
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("room-timeline").performTouchInput { swipeUp() }
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isEmpty() }
    }

    @Test
    fun commandFilterTakesNamesWithSpaces() {
        val field = compose.onNode(hasSetTextAction())
        field.performTextInput("/photo")
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("command-photo album")).fetchSemanticsNodes().isNotEmpty()
        }
        check(compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isEmpty()) {
            "带空格的名字没过滤掉别的命令"
        }
        // 空格后面接着打，还在这个名字里，列表不消失。
        field.performTextInput(" a")
        compose.waitForIdle()
        compose.onNodeWithTag("command-photo album").assertIsDisplayed()
        field.performTextClearance()
        field.performTextInput("/summarize-everything")
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("command-summarize-everything-in-this-thread"))
                .fetchSemanticsNodes().isNotEmpty()
        }
        // 长名字整条都在，不被挤没。
        compose.onNodeWithTag("command-summarize-everything-in-this-thread").assertIsDisplayed()
        compose.onNodeWithText("/summarize-everything-in-this-thread").assertIsDisplayed()
    }

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
}
