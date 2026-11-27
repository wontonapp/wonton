package app.wonton.android

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import app.wonton.ui.R
import org.junit.Test

/** 自己发的、发送失败的那条（`ui_fake_room.rs` 的 `FAILED_SEND_INDEX`）。 */
private const val FAILED_SEND = 24
/** 带一个发送失败的选项的那条（`ui_fake_room.rs` 的 `FAILED_CHOICE_INDEX`）。 */
private const val FAILED_CHOICE = 25
/** 失败原因的原文（`ui_fake_room.rs`）。 */
private const val FAILED_REASON = "Server timed out while sending"

/** 失败状态（ADR-0109 第 10 节）：被拒的选项能看原因再重发，发送失败的标记有够大的重发入口。 */
class RoomTelegramFailureTest : RoomTelegramCase() {
    @Test
    fun failedChoiceKeepsReasonInDetails() {
        scrollTo(FAILED_CHOICE)
        item(FAILED_CHOICE).performScrollTo()
        // 按钮里不再画原因，原因在下面一行小字里。
        compose.onAllNodes(hasText(FAILED_REASON) and hasAnyAncestor(hasTestTag("choice-⏳")), useUnmergedTree = true)
            .assertCountEquals(0)
        compose.onNodeWithText(string(R.string.message_choice_failed)).assertIsDisplayed()
        // 私有 Bot 的顶栏也可能带一张卡片和它自己的“详情”，按这条消息限定。
        compose.onAllNodes(
            hasText(string(R.string.common_details)) and hasAnyAncestor(hasTestTag("item-ui-fake-$FAILED_CHOICE")),
            useUnmergedTree = true,
        ).onFirst().performClick()
        compose.onNodeWithText(FAILED_REASON).assertIsDisplayed()
        screenshot("failed-choice-open")
        // 失败的按钮还能点，点了就重发。
        choice("⏳").assertIsEnabled().performClick()
        compose.waitUntil(10_000) {
            choice("⏳").fetchSemanticsNode().config[SemanticsProperties.Selected]
        }
        compose.waitUntil(10_000) { compose.onAllNodesWithText(FAILED_REASON).fetchSemanticsNodes().isEmpty() }
        screenshot("failed-choice-resent")
    }

    @Test
    fun failedMarkHasATargetSizedHitArea() {
        scrollTo(FAILED_SEND)
        item(FAILED_SEND).performScrollTo()
        val mark = compose.onNodeWithContentDescription(string(R.string.timeline_send_failed_retrying))
        val target = with(compose.density) { 48.dp.toPx() }
        val box = mark.fetchSemanticsNode().boundsInRoot
        check(box.width >= target - 1f) { "失败标记宽 ${box.width}，不到 $target" }
        check(box.height >= target - 1f) { "失败标记高 ${box.height}，不到 $target" }
        // 整个命中区都能点开重发菜单。
        mark.performClick()
        compose.onNodeWithText(string(R.string.timeline_resend)).assertIsDisplayed()
        screenshot("failed-mark-menu")
    }
}
