package app.wonton.android

import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.unit.dp
import app.wonton.ui.R
import org.junit.Test

/**
 * 最后一条给输入栏让位（#554 的 Android 半）：键盘弹着、草稿把输入栏顶高时，列表贴底，最后一条停在输入栏上方。
 * 假核心（`ui_fake_timeline.rs` 的发送）自己回显，不需要后端。
 */
class RoomComposerClearanceTest : RoomTelegramCase() {
    @Test
    fun lastMessageClearsComposer() {
        val field = compose.onNode(hasSetTextAction())
        field.performTextInput(LONG_TEXT)
        settleKeyboard()
        compose.onNodeWithContentDescription(string(R.string.composer_send)).performClick()
        val sent = compose.onAllNodes(hasTestTag("item-ui-fake-sent-1"))
        compose.waitUntil(10_000) { sent.fetchSemanticsNodes().isNotEmpty() }

        // 点发送不保证键盘留着（iOS 上会收起），点回输入框再打：不点回来，下面量到的是没键盘的可见区。
        field.performClick()
        settleKeyboard()
        val before = composerHeight()
        field.performTextInput(DRAFT)
        // 草稿把输入栏顶高才算数：量不到长高时，下面的结论和短草稿一样。
        val grown = with(compose.density) { 32.dp.toPx() }
        compose.waitUntil(10_000) { composerHeight() > before + grown }
        settleKeyboard()
        compose.waitForIdle()

        val last = checkNotNull(sent.fetchSemanticsNodes().singleOrNull()) { "最后一条不在列表里" }
        val composer = compose.onNodeWithTag("composer-block").fetchSemanticsNode().boundsInRoot
        val list = compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot
        val lastBox = last.boundsInRoot
        check(lastBox.top >= list.top) { "最后一条跑到列表上面：$lastBox < $list" }
        check(lastBox.bottom <= composer.top + 0.5f) {
            "最后一条藏进输入栏：底边 ${lastBox.bottom} 越过输入栏上沿 ${composer.top}（输入栏高 ${composer.height}）"
        }
        screenshot("last-message-clears-composer")
    }

    private fun composerHeight(): Float =
        compose.onNodeWithTag("composer-block").fetchSemanticsNode().boundsInRoot.height
}

/** 发出去的长消息：发送键出现得先有字。 */
private val LONG_TEXT = "long reply text ".repeat(30)

/** 之后的长草稿：顶到输入栏的行数上限，把输入栏撑高。 */
private val DRAFT = "draft ".repeat(40)
