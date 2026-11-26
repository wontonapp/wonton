package app.wonton.android

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.unit.dp
import app.wonton.ui.R
import org.junit.Test

/** 回复与引用（ADR-0109 第 5 节）：左滑起回复、点引用跳过去再退回来、引用块只包住自己的内容。 */
class RoomTelegramReplyTest : RoomTelegramCase() {
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
        compose.waitForIdle()
        screenshot("after-quote-click")
        compose.waitUntil(10_000) { visible(5) && !visible(21) }
        screenshot("jump-highlight")
        // 离开底部时“向下”先退回跳之前的那条，不直接回最新。
        compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom), substring = true).performClick()
        compose.waitUntil(10_000) { visible(21) }
        screenshot("jump-back")
    }

    @Test
    fun quoteBlockHugsItsContent() {
        scrollTo(LONG_REPLY)
        item(LONG_REPLY).performScrollTo()
        val quote = compose.onNode(
            hasTestTag("quote") and hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        )
        val box = quote.fetchSemanticsNode().boundsInRoot
        val name = compose.onNode(
            hasText("Owner") and hasAnyAncestor(hasTestTag("quote")) and
                hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot
        val summary = compose.onNode(
            hasText(QUOTED_TEXT.trim()) and hasAnyAncestor(hasTestTag("quote")) and
                hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot
        val slack = with(compose.density) { 12.dp.toPx() }
        // 引用块只占名字加内容本身那么高：内容就是两行，多出来的就是被拉高的。
        check(summary.bottom >= name.bottom) { "引用块的正文跑到名字上面：${summary.bottom} < ${name.bottom}" }
        check(box.height <= name.height + summary.height + slack) {
            "引用块被拉高：${box.height} > 名字 ${name.height} + 内容 ${summary.height} + $slack"
        }
        check(box.bottom - summary.bottom <= slack) { "引用块底下多出 ${box.bottom - summary.bottom}" }
    }
}
