@file:Suppress("MagicNumber")

package app.wonton.ui

import app.wonton.protocol.ImageContent
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.MessageContent
import app.wonton.protocol.MessageGroup
import app.wonton.protocol.MessageRun
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import kotlinx.coroutines.CoroutineScope
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import kotlin.coroutines.EmptyCoroutineContext

/**
 * 同组照片翻页的判定：松手按四分之一屏或轻弹速度翻页，到头回弹不循环；到头的方向拖动按橡皮筋跟手。
 * 翻页动画本身（`Animatable` 要帧时钟）不在单测里跑。
 */
class ImageViewerPagingTest {
    private val width = 1000f
    private val fling = 400f

    @Test
    fun dragPastQuarterScreenTurnsThePage() {
        assertEquals(0, pageTarget(1, 3, 300f, 0f, width, fling))
        assertEquals(2, pageTarget(1, 3, -300f, 0f, width, fling))
        assertEquals(1, pageTarget(1, 3, 200f, 0f, width, fling))
        assertEquals(1, pageTarget(1, 3, -200f, 0f, width, fling))
    }

    @Test
    fun quickFlickTurnsThePageEvenWhenShort() {
        assertEquals(0, pageTarget(1, 3, 50f, 500f, width, fling))
        assertEquals(2, pageTarget(1, 3, -50f, -500f, width, fling))
        // 速度方向和拉动方向相反时不翻。
        assertEquals(1, pageTarget(1, 3, 50f, -500f, width, fling))
    }

    @Test
    fun bounceAtTheEndsInsteadOfWrapping() {
        assertEquals(0, pageTarget(0, 3, 900f, 5000f, width, fling))
        assertEquals(2, pageTarget(2, 3, -900f, -5000f, width, fling))
    }

    @Test
    fun dragFollowsWithRubberBandAtTheEnds() {
        val middle = PageState(3, 1, { width }, fling, CoroutineScope(EmptyCoroutineContext))
        middle.applyDrag(-100f)
        assertEquals(-100f, middle.drag, 0.01f)
        assertEquals(2, middle.neighborIndex)
        // 最后一张还往左拉：位移减半，没有邻页可画；往回拉不减速。
        val last = PageState(3, 2, { width }, fling, CoroutineScope(EmptyCoroutineContext))
        last.applyDrag(-100f)
        assertEquals(-50f, last.drag, 0.01f)
        assertNull(last.neighborIndex)
        last.applyDrag(40f)
        assertEquals(-10f, last.drag, 0.01f)
    }

    @Test
    fun returnStrokeOnTheFirstPageIsNotDamped() {
        val first = PageState(3, 0, { width }, fling, CoroutineScope(EmptyCoroutineContext))
        first.applyDrag(-100f)
        first.applyDrag(30f)
        assertEquals(-70f, first.drag, 0.01f)
        first.applyDrag(30f)
        assertEquals(-40f, first.drag, 0.01f)
        // 跨进橡皮筋区的那一段开始减速。
        first.applyDrag(30f)
        assertEquals(-10f, first.drag, 0.01f)
        first.applyDrag(30f)
        assertEquals(5f, first.drag, 0.01f)
        first.applyDrag(30f)
        assertEquals(20f, first.drag, 0.01f)
    }

    /** 真绑定上的收组：乱序进来的同组三张按 index 排好；没组的单张不进。 */
    @Test
    fun imageGroupsCollectsOneSendsPhotos() {
        val owner = "@owner:home.example"
        val echo = "@echo:home.example"
        val photo = { index: Int, sender: String ->
            val handle = MediaHandle("ui-fake-photo-$index")
            TimelineItem(
                "ui-fake-$index",
                TimelineContent.Message(
                    TimelineMessage(
                        "\$ui-fake-$index",
                        sender,
                        if (sender == owner) "Owner" else "Echo",
                        sender == owner,
                        1_792_000_000_000uL + index.toULong() * 60_000uL,
                        SendState.Sent,
                        MessageContent.Image(
                            ImageContent(handle, null, "photo-$index.bmp", "image/bmp", null, 320uL, 240uL, null),
                        ),
                        false,
                        emptyList(),
                        null,
                        null,
                        null,
                        emptyList(),
                        MessageGroup("000102030405060708090a0b0c0d0e0f", index.toUInt(), 3u),
                        null,
                        false,
                        MessageRun.ALONE,
                        null,
                        null,
                        null,
                    ),
                ),
            )
        }
        val solo = TimelineItem(
            "ui-fake-solo",
            TimelineContent.Message(
                TimelineMessage(
                    "\$ui-fake-solo",
                    "@echo:home.example",
                    "Echo",
                    false,
                    1_792_000_000_000uL,
                    SendState.Sent,
                    MessageContent.Image(
                        ImageContent(MediaHandle("solo"), null, "solo.bmp", "image/bmp", null, 320uL, 240uL, null),
                    ),
                    false,
                    emptyList(),
                    null,
                    null,
                    null,
                    emptyList(),
                    null,
                    null,
                    false,
                    MessageRun.ALONE,
                    null,
                    null,
                    null,
                ),
            ),
        )
        // 同房间两个人撞同一个组 id：各算各的组。
        val groups = listOf(photo(2, owner), solo, photo(1, owner), photo(0, owner), photo(9, echo)).imageGroups()
        val id = "000102030405060708090a0b0c0d0e0f"
        assertEquals(setOf(owner to id, echo to id), groups.keys)
        assertEquals(
            listOf("ui-fake-photo-0", "ui-fake-photo-1", "ui-fake-photo-2"),
            groups.getValue(owner to id).map { it.media.token },
        )
        assertEquals(listOf("ui-fake-photo-9"), groups.getValue(echo to id).map { it.media.token })
    }
}
