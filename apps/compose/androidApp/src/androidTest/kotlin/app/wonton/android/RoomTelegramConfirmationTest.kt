package app.wonton.android

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import app.wonton.ui.R
import org.junit.Test

/** 待确认入口：条里写清是哪张卡，点开就能看详情。 */
class RoomTelegramConfirmationTest : RoomTelegramCase() {
    @Test
    fun confirmationBarNamesTheCardAndOpensIt() {
        val bar = compose.onNodeWithTag("room-confirmations")
        bar.assertIsDisplayed()
        val target = with(compose.density) { 48.dp.toPx() }
        check(bar.fetchSemanticsNode().boundsInRoot.height >= target - 1f) { "确认入口不到 48dp 高" }
        // 写清是哪一张：一行“有事要你定”，下一行是首条的标题。
        compose.onNode(
            hasText(string(R.string.confirmation_card_requests)) and hasAnyAncestor(hasTestTag("room-confirmations")),
            useUnmergedTree = true,
        ).assertIsDisplayed()
        compose.onNodeWithText("Echo wants to install Home Hub", useUnmergedTree = true).assertIsDisplayed()
        screenshot("room-confirmations")
        bar.performClick()
        compose.onNodeWithText("Home Hub can read and write everything on the disks you tick.")
            .assertIsDisplayed()
        screenshot("room-confirmations-open")
    }
}
