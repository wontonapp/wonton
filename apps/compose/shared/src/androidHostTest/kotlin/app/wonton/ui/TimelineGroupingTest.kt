package app.wonton.ui

import app.wonton.protocol.ImageCaption
import app.wonton.protocol.ImageContent
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.MessageContent
import app.wonton.protocol.MessageGroup
import app.wonton.protocol.MessageRun
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TimelineGroupingTest {
    @Test
    fun adjacentImagesInSameSendAreFlushIncludingCaptionOnLastImage() {
        val first = message(image())
        val next = message(image(ImageCaption("caption", emptyList())), 1u)

        assertTrue(first.row().flushesWith(next.row()))
    }

    @Test
    fun textAndImageNeverFlushInEitherOrderEvenWithSameSenderAndGroup() {
        val text = message(MessageContent.Text("text", emptyList(), emptyList()))
        val photo = message(image(), 1u)

        assertFalse(text.row().flushesWith(photo.row()))
        assertFalse(photo.row().flushesWith(text.row()))
    }

    @Test
    fun missingOrDifferentGroupSeparatesImages() {
        val first = message(image())
        val next = message(image(), 1u)
        val otherGroup = MessageGroup("11111111111111111111111111111111", 1u, 2u)

        assertFalse(first.copy(group = null).row().flushesWith(next.row()))
        assertFalse(first.row().flushesWith(next.copy(group = null).row()))
        assertFalse(first.row().flushesWith(next.copy(group = otherGroup).row()))
        assertFalse(first.row().flushesWith(null))
    }

    @Test
    fun differentSenderSeparatesImages() {
        val first = message(image())
        val next = message(image(), 1u)

        assertFalse(first.row().flushesWith(next.copy(senderId = "@other:example.org").row()))
    }

    @Test
    fun runJoinsOnlyInsideOneGroup() {
        val first = message(image(), run = MessageRun.FIRST)
        val middle = message(image(), 1u, run = MessageRun.MIDDLE)
        val last = message(image(), 2u, run = MessageRun.LAST)

        assertTrue(middle.row().joins(first.row()))
        assertTrue(last.row().joins(middle.row()))
        assertFalse(first.row().joins(null))
        // 组首接在组首后面、组中接在自己后面都不算接着。
        assertFalse(first.row().joins(first.row()))
        assertFalse(last.row().joins(last.row()))
    }

    @Test
    fun aloneOrOtherNeverJoin() {
        val first = message(image(), run = MessageRun.FIRST)
        val other = message(image(), 1u, run = MessageRun.OTHER)

        assertFalse(message(image(), 1u).row().joins(first.row()))
        assertFalse(other.row().joins(first.row()))
        assertFalse(message(image(), 2u, run = MessageRun.LAST).row().joins(other.row()))
        val stranger = message(image(), 2u, run = MessageRun.LAST).copy(senderId = "@other:example.org")
        assertFalse(stranger.row().joins(first.row()))
    }

    @Test
    fun captionOnEarlierImageKeepsSpaceBeforeFollowingImage() {
        val first = message(image(ImageCaption("caption", emptyList())))
        val next = message(image(), 1u)

        assertFalse(first.row().flushesWith(next.row()))
    }

    private fun image(caption: ImageCaption? = null): MessageContent = MessageContent.Image(
        ImageContent(
            media = MediaHandle("test-media"),
            thumbnail = null,
            filename = "photo.jpg",
            mimeType = "image/jpeg",
            size = null,
            width = null,
            height = null,
            caption = caption,
        ),
    )

    private fun message(body: MessageContent, index: UInt = 0u, run: MessageRun = MessageRun.ALONE) = TimelineMessage(
        eventId = null,
        senderId = "@sender:example.org",
        senderName = "Sender",
        isOwn = true,
        timestampMs = 0uL,
        sendState = SendState.Sent,
        body = body,
        edited = false,
        readBy = emptyList(),
        reply = null,
        sideChat = null,
        preview = null,
        actions = emptyList(),
        group = MessageGroup("00000000000000000000000000000000", index, 2u),
        run = run,
    )

    private fun TimelineMessage.row() = TimelineItem(group?.index.toString(), TimelineContent.Message(this))
}
