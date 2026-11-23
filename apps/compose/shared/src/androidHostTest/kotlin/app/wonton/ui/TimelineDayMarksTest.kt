package app.wonton.ui

import app.wonton.protocol.MessageContent
import app.wonton.protocol.MessageRun
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import org.junit.Assert.assertEquals
import org.junit.Test

class TimelineDayMarksTest {
    @Test
    fun dividerWritesTheCrossDayDateItself() {
        val items = listOf(messageAt(DAY_B), dividerAt(DAY_B), messageAt(DAY_A))

        // 只有最顶上那条（是它那天的第一条）自己带一行，跨天处由分隔条目写，不再叠一行。
        assertEquals(listOf(null, null, dateLabel(DAY_A)), dayMarksFor(items).toList())
    }

    @Test
    fun boundaryWithoutDividerStillGetsARow() {
        val items = listOf(messageAt(DAY_B), messageAt(DAY_A))

        // 没有分隔条目时，跨天边界照旧由壳兜底出一行。
        assertEquals(listOf(dateLabel(DAY_B), dateLabel(DAY_A)), dayMarksFor(items).toList())
    }

    @Test
    fun noteWithoutTimestampFollowsTheStateAroundIt() {
        val items = listOf(messageAt(DAY_B), note("Agent 改了个名字"), dividerAt(DAY_B), messageAt(DAY_A))

        // 旁注没有时间，跟着更新（下标更小）的那条算同一天；那一天的分界由分隔条目写，旁注不再叠。
        assertEquals(listOf(null, null, null, dateLabel(DAY_A)), dayMarksFor(items).toList())
    }

    @Test
    fun topMessageMarksItsOwnDay() {
        assertEquals(listOf(dateLabel(DAY_A)), dayMarksFor(listOf(messageAt(DAY_A))).toList())
    }

    @Test
    fun sameDayNeighboursMarkNothing() {
        val items = listOf(messageAt(DAY_A + 7_200_000L), messageAt(DAY_A + 3_600_000L), messageAt(DAY_A))

        assertEquals(listOf(null, null, dateLabel(DAY_A)), dayMarksFor(items).toList())
    }

    @Test
    fun startAtTheTopRidesOnItsDaysDivider() {
        val items = listOf(messageAt(DAY_A), dividerAt(DAY_A), timelineStart())

        // 翻到房间开头：最顶上也是最后一条是 TimelineStart，它下面紧挨着同一天的分隔条目，不再出兜底那行。
        assertEquals(listOf(null, null, null), dayMarksFor(items).toList())
    }

    private fun messageAt(timestampMs: Long) = TimelineItem(
        id = "m$timestampMs",
        content = TimelineContent.Message(
            TimelineMessage(
                eventId = null,
                senderId = "@sender:example.org",
                senderName = "Sender",
                isOwn = false,
                timestampMs = timestampMs.toULong(),
                sendState = SendState.Sent,
                body = MessageContent.Text("text", emptyList(), emptyList()),
                edited = false,
                readBy = emptyList(),
                reply = null,
                sideChat = null,
                preview = null,
                actions = emptyList(),
                group = null,
                run = MessageRun.ALONE,
            ),
        ),
    )

    private fun dividerAt(timestampMs: Long) = TimelineItem(
        id = "d$timestampMs",
        content = TimelineContent.DateDivider(timestampMs.toULong()),
    )

    private fun timelineStart() = TimelineItem("start", TimelineContent.TimelineStart)

    private fun note(text: String) = TimelineItem("n", TimelineContent.Notice(text))

    private companion object {
        const val DAY_A = 20_000L * 86_400_000L
        const val DAY_B = DAY_A + 86_400_000L
    }
}
