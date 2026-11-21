@file:Suppress("MagicNumber")

package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import app.wonton.protocol.MessageContent
import app.wonton.protocol.MessageRun
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem
import app.wonton.protocol.TimelineMessage
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.text.DateFormat
import java.util.Date

/**
 * 跨天那里的固定假数据：前一天最后两条、一条日期分隔条目、后一天头两条，新到旧排（列表就是这么排的）。
 * 空白没有核心可用，消息气泡只画文字和时间，日期行用界面自己的 [DatePill] 和 [DaySeparator]。
 */
private const val DAY_A = 1_793_520_000_000L
private const val DAY_B = DAY_A + 86_400_000L

private fun message(id: String, text: String, at: Long, own: Boolean) = TimelineItem(
    id,
    TimelineContent.Message(
        TimelineMessage(
            eventId = null,
            senderId = if (own) "@me:example.org" else "@ann:example.org",
            senderName = if (own) "Me" else "Ann",
            isOwn = own,
            timestampMs = at.toULong(),
            sendState = SendState.Sent,
            body = MessageContent.Text(text, emptyList(), emptyList()),
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

private fun divider(at: Long) = TimelineItem(
    "day-break",
    TimelineContent.DateDivider(timestampMs = at.toULong()),
)

@Preview(name = "跨天一行日期", locale = "en", widthDp = 390, heightDp = 300)
@Composable
internal fun DayBreakPreview() {
    PreviewFrame {
        DayBreakRows()
    }
}

/** 深色下同一处：胶囊的底和字跟着换，仍只有一行。 */
@Preview(name = "跨天一行日期（深色）", locale = "en", widthDp = 390, heightDp = 300)
@Composable
internal fun DayBreakDarkPreview() {
    PreviewFrame(dark = true) {
        DayBreakRows()
    }
}

@Composable
private fun DayBreakRows() {
    val items = listOf(
        message("b2", "First after midnight", DAY_B + 60_000, own = false),
        message("b1", "Still up?", DAY_B, own = true),
        divider(DAY_B),
        message("a2", "Talk tomorrow", DAY_A + 60_000, own = false),
        message("a1", "Night", DAY_A, own = true),
    )
    val marks = dayMarksFor(items)
    Column(
        Modifier.fillMaxWidth().padding(vertical = WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.runGap),
    ) {
        // 列表新到旧，画的时候倒过来才是从上到下越来越新的样子。
        for (index in items.indices.reversed()) {
            marks[index]?.let { label -> DaySeparator(label, Modifier.testTag("day-separator")) }
            DayBreakRow(items[index])
        }
    }
}

@Composable
private fun DayBreakRow(item: TimelineItem) {
    if (item.content is TimelineContent.DateDivider) {
        DaySeparator(
            dateLabel(((item.content as TimelineContent.DateDivider).timestampMs).toLong()),
            Modifier.testTag("day-divider"),
        )
        return
    }
    val message = (item.content as TimelineContent.Message).message
    val colors = WontonTheme.colors
    val fill = if (message.isOwn) colors.accentFill else colors.surfaceSubtle
    val ink = if (message.isOwn) colors.textOnAccentFill else colors.textPrimary
    Column(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.s16),
        horizontalAlignment = if (message.isOwn) Alignment.End else Alignment.Start,
    ) {
        Column(
            Modifier
                .widthIn(max = 280.dp)
                .background(fill.toColor(), RoundedCornerShape(WontonTheme.radii.bubble))
                .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8),
        ) {
            Text(
                (message.body as MessageContent.Text).body,
                style = WontonTheme.typography.body,
                color = ink.toColor(),
            )
            Text(
                DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(message.timestampMs.toLong())),
                style = WontonTheme.typography.caption,
                color = ink.toColor().copy(alpha = 0.6f),
            )
        }
    }
}
