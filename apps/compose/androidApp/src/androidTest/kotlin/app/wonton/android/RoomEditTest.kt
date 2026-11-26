package app.wonton.android

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import app.wonton.protocol.Action
import app.wonton.protocol.SettingsAction
import app.wonton.protocol.ThemeMode
import app.wonton.ui.R
import app.wonton.ui.send
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** 自己发的普通文字（`ui_fake_room.rs` 第 15 条）：菜单里给“编辑”。 */
private const val OWN = 15
/** Echo 发的（第 14 条）：菜单里没有“编辑”。 */
private const val FOREIGN = 14
private const val ORIGINAL = "Fixture message $OWN"
private const val DRAFT = "draft typed before editing"
private const val REPLACEMENT = "Fixture edited text"

/** 编辑消息（ADR-0109 第 10 节）：菜单“编辑”、输入框上方的编辑条、取消恢复草稿、确认换正文。 */
class RoomEditTest : RoomTelegramCase() {
    @Test
    fun editShowsBarAndCancelRestoresDraft() {
        scrollTo(OWN)
        item(OWN).performScrollTo()
        compose.onNode(hasSetTextAction()).performTextInput(DRAFT)
        compose.waitUntil(5_000) { fieldText() == DRAFT }

        openEdit(OWN)
        compose.onNodeWithTag("composer-edit-bar").assertIsDisplayed()
        compose.onNode(hasTestTag("edit-summary"), useUnmergedTree = true).assertTextEquals(ORIGINAL)
        assertEquals("输入框没换成原文", ORIGINAL, fieldText())
        compose.onNodeWithContentDescription(string(R.string.composer_edit_confirm)).assertIsDisplayed()
        compose.waitForIdle()
        Thread.sleep(500)
        screenshot("edit-bar")
        darkShot("edit-bar-dark")

        compose.onNode(
            hasContentDescription(string(R.string.common_close)) and hasAnyAncestor(hasTestTag("composer-edit-bar")),
        ).performClick()
        compose.waitUntil(5_000) { fieldText() == DRAFT }
        assertEquals(0, compose.onAllNodes(hasTestTag("composer-edit-bar")).fetchSemanticsNodes().size)
        assertTrue(compose.onAllNodesWithText(ORIGINAL, substring = true).fetchSemanticsNodes().isNotEmpty())
        assertEquals(
            0,
            compose.onAllNodes(hasTestTag("message-edited"), useUnmergedTree = true).fetchSemanticsNodes().size,
        )
        assertTrue(
            compose.onAllNodesWithContentDescription(string(R.string.composer_send)).fetchSemanticsNodes().isNotEmpty(),
        )
    }

    @Test
    fun foreignMessageMenuHasNoEdit() {
        scrollTo(FOREIGN)
        item(FOREIGN).performScrollTo()
        bubble(FOREIGN).performTouchInput { longClick() }
        compose.onNodeWithText(string(R.string.message_reply)).assertIsDisplayed()
        assertEquals(0, compose.onAllNodesWithText(string(R.string.message_edit)).fetchSemanticsNodes().size)
        compose.onNodeWithText(string(R.string.message_reply)).performClick()
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
    }

    @Test
    fun confirmReplacesTextAndShowsEdited() {
        scrollTo(OWN)
        item(OWN).performScrollTo()
        openEdit(OWN)
        compose.waitUntil(5_000) { fieldText() == ORIGINAL }
        compose.onNode(hasSetTextAction()).performTextClearance()
        compose.onNode(hasSetTextAction()).performTextInput(REPLACEMENT)

        compose.onNodeWithContentDescription(string(R.string.composer_edit_confirm)).performClick()
        // 编辑途中打过字，原草稿就不盖回来了（同核心）：新正文留在输入框里。
        compose.waitUntil(10_000) {
            fieldText() == REPLACEMENT &&
                compose.onAllNodes(hasTestTag("composer-edit-bar")).fetchSemanticsNodes().isEmpty()
        }
        // 退出编辑以后那颗键换回发送。
        compose.waitUntil(5_000) {
            compose.onAllNodesWithContentDescription(string(R.string.composer_send)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText(REPLACEMENT, substring = true).fetchSemanticsNodes().isNotEmpty()
        }
        assertEquals(0, compose.onAllNodesWithText(ORIGINAL, substring = true).fetchSemanticsNodes().size)
        compose.waitUntil(5_000) {
            compose.onAllNodes(hasTestTag("message-edited"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.waitForIdle()
        Thread.sleep(500)
        screenshot("edit-sent")
        darkShot("edit-sent-dark")
    }

    /** 长按那条消息点“编辑”，等编辑条出来。 */
    private fun openEdit(index: Int) {
        bubble(index).performTouchInput { longClick() }
        compose.onNodeWithText(string(R.string.message_edit)).performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("composer-edit-bar")).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun fieldText(): String =
        compose.onNode(hasSetTextAction()).fetchSemanticsNode().config[SemanticsProperties.EditableText].text

    /** 同一处再拍一张深色的，拍完换回浅色。主题走核心的设置，和 App 里切换的是同一条路。 */
    private fun darkShot(name: String) {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        core.send(Action.Settings(SettingsAction.SetThemeMode(ThemeMode.DARK)))
        assertEquals(ThemeMode.DARK, core.appearance().themeMode)
        // 外观走订阅，等它到界面再拍；截图看不出重排有没有跑完，留一点时间。
        compose.waitForIdle()
        Thread.sleep(500)
        compose.waitForIdle()
        screenshot(name)
        core.send(Action.Settings(SettingsAction.SetThemeMode(ThemeMode.LIGHT)))
        assertEquals(ThemeMode.LIGHT, core.appearance().themeMode)
        compose.waitForIdle()
        Thread.sleep(500)
        compose.waitForIdle()
    }
}
