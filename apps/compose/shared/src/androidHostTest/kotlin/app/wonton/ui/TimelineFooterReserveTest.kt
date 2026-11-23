package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import app.wonton.protocol.MessageContent
import app.wonton.protocol.MessageGroup
import app.wonton.protocol.MessageRun
import app.wonton.protocol.MessageStatus
import app.wonton.protocol.RichBlock
import app.wonton.protocol.RichBlockKind
import app.wonton.protocol.RichSpan
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineMessage
import java.text.DateFormat
import java.util.Date
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/** 脚注压在最后一行上的透明占位（#790 已修，这里锁住）：回执那一格必须是不折行的空白。 */
class TimelineFooterReserveTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5,
        theme = "android:Theme.Material.Light.NoActionBar",
    )

    private val resources by lazy { paparazzi.resources }

    @Test
    fun readReceiptLeavesAnUnbreakableGapAfterTheTime() {
        val reserve = checkNotNull(footer(MessageStatus.READ))

        assertTrue("开头要留一个能折行的普通空格：$reserve", reserve.startsWith(" "))
        val gap = reserve.substringAfter(TIME_UNBROKEN)
        assertGap(gap, reserve)
    }

    @Test
    fun editedMarkComesBeforeTheTimeAndKeepsTheGap() {
        val reserve = checkNotNull(footer(MessageStatus.SENT, edited = true))

        val edited = resources.getString(R.string.timeline_edited)
        assertTrue("没有编辑标记：$reserve", reserve.contains(edited))
        assertTrue("编辑标记排到时间后面了：$reserve", reserve.indexOf(edited) < reserve.indexOf(TIME_UNBROKEN))
        assertGap(reserve.substringAfter(TIME_UNBROKEN), reserve)
    }

    /** 时间后面至少 6 个不折行空格（回执图标 14dp，一个字宽不够），并且不再有别的字。 */
    private fun assertGap(gap: String, reserve: String) {
        assertTrue("时间后面没有足够的空白，图标会压到字：$reserve", gap.length >= GAP_FLOOR)
        assertTrue("空白里混进了别的字：$reserve", gap.all { it == '\u00A0' })
    }

    @Test
    fun otherPeoplesMessagesLeaveNoReceiptGap() {
        val reserve = checkNotNull(footer(MessageStatus.SENT, own = false))
        // 时间前面是“已编辑”时，时间后面的分隔也是不折行空格；回执那一格只看时间之后。
        val edited = checkNotNull(footer(MessageStatus.SENT, edited = true, own = false))

        val mark = resources.getString(R.string.timeline_edited)
        assertTrue("别人的消息不该留回执的空白：$reserve", reserve.substringAfter(TIME_UNBROKEN).isBlank())
        assertTrue("编辑标记排到时间后面了：$edited", edited.indexOf(mark) < edited.indexOf(TIME_UNBROKEN))
        assertTrue("别人的消息不该留回执的空白：$edited", edited.substringAfter(TIME_UNBROKEN).isBlank())
    }

    @Test
    fun messagesWithoutATextLineHaveNoReserve() {
        assertNull(footer(MessageStatus.READ, body = MessageContent.Redacted))
    }

    private fun footer(
        status: MessageStatus?,
        edited: Boolean = false,
        own: Boolean = true,
        body: MessageContent = paragraph(),
    ): String? = footerReserve(
        TimelineMessage(
            eventId = "1",
            senderId = "@sender:example.org",
            senderName = "Sender",
            isOwn = own,
            timestampMs = AT_MS.toULong(),
            sendState = SendState.Sent,
            status = status,
            body = body,
            edited = edited,
            readBy = emptyList(),
            reply = null,
            sideChat = null,
            preview = null,
            actions = emptyList(),
            group = MessageGroup("00000000000000000000000000000000", 0u, 1u),
            receipt = null,
            startsTimeGroup = false,
            run = MessageRun.ALONE,
            choices = null,
            senderAvatar = null,
        ),
        resources,
    )

    private fun paragraph() = MessageContent.Text(
        body = "text",
        blocks = listOf(
            RichBlock(
                kind = RichBlockKind.Paragraph,
                depth = 0u,
                quoted = false,
                spans = listOf(
                    RichSpan(text = "text", bold = false, italic = false, code = false, strike = false, link = null),
                ),
            ),
        ),
        files = emptyList(),
    )

    private companion object {
        const val AT_MS = 1_700_000_000_000L

        /** 回执图标 14dp 比一个字宽，时间后面至少留 6 个不折行空格。 */
        const val GAP_FLOOR = 6
        val TIME: String = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(AT_MS))

        /** 脚注里时间内部的空格也换成了不折行空格，按这个找时间。 */
        val TIME_UNBROKEN: String = TIME.replace(' ', '\u00A0')
    }
}
