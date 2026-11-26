package app.wonton.android

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import app.wonton.ui.R
import org.junit.Test

/** 房间顶栏（ADR-0109 第 4 节）：第一条气泡不被顶栏盖住、状态行不压气泡、标题和头像能进资料页。 */
class RoomTelegramRoomHeaderTest : RoomTelegramCase() {
    @Test
    fun groupTopKeepsFirstBubbleBelowTheBar() {
        openFamily()
        // 26 是最新的一条，往上甩到列表头上看它。
        scrollTo(26)
        repeat(3) {
            compose.onNodeWithTag("room-timeline").performTouchInput {
                swipe(Offset(centerX, centerY * 1.9f), Offset(centerX, centerY * 0.3f), durationMillis = 400)
            }
            compose.waitForIdle()
        }
        bubble(26).assertIsDisplayed()
        // 最早那条气泡的顶边还在顶栏下面，没被顶栏拿走。
        val first = bubble(26).fetchSemanticsNode().boundsInRoot
        // 顶栏和它下面那行（群里的 AI 等）都不能压在第一条上。
        val bar = compose.onNodeWithTag("room-bar").fetchSemanticsNode().boundsInRoot.bottom
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot.bottom
        val limit = maxOf(bar, header)
        check(first.top >= limit) { "第一条被顶栏盖住：${first.top} < $limit" }
    }

    @Test
    fun pluginStatusRowCoversNoBubble() {
        // 群里那行 AI（假房间里是 Hermes）自己占一条，时间线让开它，气泡不压在它上面。
        openFamily()
        compose.onNodeWithText("Hermes", useUnmergedTree = true).assertIsDisplayed()
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true).fetchSemanticsNodes()
        check(bubbles.isNotEmpty())
        for (bubble in bubbles) {
            val box = bubble.boundsInRoot
            check(!box.overlaps(header)) { "气泡压在状态行上：$box vs $header" }
        }
    }

    @Test
    fun avatarOpensBotProfileTitleIsDisplayOnly() {
        val profile = compose.onNodeWithTag("bot-profile")
        // 入口只剩头像一个，顶栏里没有额外的信息按钮，名字也不再是第二个入口。
        compose.onAllNodesWithContentDescription(string(R.string.bot_profile_open)).assertCountEquals(1)
        compose.onNodeWithTag("roomTopBarTitle").performClick()
        compose.waitForIdle()
        profile.assertDoesNotExist()
        // 资料页自己的返回按钮跟页面一起出现；系统返回按早了会把整个 Activity 关掉。
        compose.onNodeWithTag("roomTopBarAvatar").performClick()
        profile.assertIsDisplayed()
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        screenshot("bar-profile")
    }
}
