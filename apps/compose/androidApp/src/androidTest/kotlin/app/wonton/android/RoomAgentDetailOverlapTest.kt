package app.wonton.android

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import org.junit.Test

/**
 * AI 管家错误卡展开详情（#701）：原因再长也只能在卡里滚，卡片长高不能把聊天挤出屏幕。
 * 假房间的 Echo 带着一条够长的降级原因；照片和文字各看一条。
 */
class RoomAgentDetailOverlapTest : RoomTelegramCase() {
    @Test
    fun longDetailKeepsPhotoVisible() = detailKeepsItemVisible(PHOTO_INDEX)

    @Test
    fun longDetailKeepsTextVisible() = detailKeepsItemVisible(TEXT_INDEX)

    private fun detailKeepsItemVisible(index: Int) {
        pinToTop(index)
        val before = visibleFraction(index)
        check(before >= 0.5f) { "第 $index 条没先露出来，展开前后对比不算数：$before" }

        // 有测试标识按标识点，旧包没有就按卡片里的文案点：同一个用例能在修前修后都跑。
        // 消息正文里也可能有“Details”，文案那套按顶栏限定。
        if (compose.onAllNodes(hasTestTag(DETAIL_TOGGLE)).fetchSemanticsNodes().isEmpty()) {
            compose.onAllNodes(
                hasText(string(app.wonton.ui.R.string.common_details)) and hasAnyAncestor(hasTestTag("room-header")),
                useUnmergedTree = true,
            ).onFirst().performClick()
        } else {
            compose.onNodeWithTag(DETAIL_TOGGLE).performClick()
        }
        compose.waitForIdle()
        compose.onNodeWithText(DETAIL_FIRST_LINE, substring = true).assertIsDisplayed()

        // 卡片照旧自己占一行，不压在列表上。
        val list = listBounds()
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot
        check(header.bottom <= list.top + 1f) { "卡片压到列表上了：卡底 ${header.bottom} > 列表顶 ${list.top}" }
        // 展开后这一条还得露着：卡片长高顶多把它往上推一截，不能整条推进卡片底下。
        val after = visibleFraction(index)
        check(after >= 0.5f) { "展开详情把第 $index 条盖掉一半以上（还剩 ${after * 100}%）：列表 $list" }
        screenshot("agent-detail-keeps-$index")
    }

    /** 滚到这一条，再用手指往上推，直到它贴着列表上沿（留 8px）。 */
    private fun pinToTop(index: Int) {
        scrollTo(index)
        compose.waitForIdle()
        repeat(6) {
            val list = listBounds()
            val box = item(index).fetchSemanticsNode().boundsInRoot
            val delta = box.top - list.top - 8f
            if (delta <= 24f) return
            val distance = minOf(delta, 600f)
            compose.onNodeWithTag("room-timeline").performTouchInput {
                swipe(
                    Offset(centerX, centerY + distance / 2),
                    Offset(centerX, centerY - distance / 2),
                    durationMillis = 600,
                )
            }
            compose.waitForIdle()
        }
    }

    /** 这一条落在列表里的那部分占它自身多高；整条被移出视口（列表把它回收了）算 0。 */
    private fun visibleFraction(index: Int): Float {
        val node = compose.onAllNodes(hasTestTag("item-ui-fake-$index"), useUnmergedTree = true)
            .fetchSemanticsNodes().singleOrNull() ?: return 0f
        val box = node.boundsInRoot
        val list = listBounds()
        val shown = minOf(box.bottom, list.bottom) - maxOf(box.top, list.top)
        return (shown / box.height).coerceIn(0f, 1f)
    }

    private fun listBounds(): Rect =
        compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot

    private companion object {
        const val DETAIL_TOGGLE = "agent-detail-toggle"
        const val DETAIL_FIRST_LINE = "连接模型服务失败"

        /** 一次发的三张照片里的第一张。 */
        const val PHOTO_INDEX = 5

        /** 一条普通文字消息。 */
        const val TEXT_INDEX = 25
    }
}
