package app.wonton.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.QuotedContent
import app.wonton.protocol.ReplyQuote
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.WontonIcon

/** 引用的是哪条消息：回复状态、草稿都靠它。 */
val ReplyQuote.eventId: String
    get() = when (this) {
        is ReplyQuote.Loading -> eventId
        is ReplyQuote.Ready -> message.eventId
        is ReplyQuote.Unavailable -> eventId
    }

/**
 * 消息上方引用的原消息（ADR-0073 第 2 节）：按类别画，不自己摘要。`onTint` 是自己的气泡里的前景色，`null` 用正文的墨色。
 */
@Composable
fun QuotedMessageRow(quote: ReplyQuote, core: ClientCore, modifier: Modifier = Modifier, onTint: Color? = null) {
    val colors = WontonTheme.colors
    val name = onTint ?: colors.textPrimary.toColor()
    val secondary = (onTint ?: colors.textSecondary.toColor()).copy(alpha = WontonTheme.ratio.secondaryContent)
    Row(
        modifier.fillMaxWidth().height(IntrinsicSize.Min),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Box(Modifier.width(WontonTheme.size.quoteBar).fillMaxHeight().background(secondary, RoundedCornerShape(1.dp)))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            when (quote) {
                is ReplyQuote.Ready -> {
                    Text(
                        quote.message.senderName,
                        style = WontonTheme.typography.caption,
                        color = name,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    QuotedContentLine(quote.message.content, core, secondary)
                }
                // 核心正在向服务器取原消息，取到之前画一条占位。
                is ReplyQuote.Loading -> Box(
                    Modifier.padding(vertical = WontonTheme.spacing.s4)
                        .fillMaxWidth(LOADING_FILL)
                        .height(WontonTheme.size.iconTiny)
                        .background(secondary, PLACEHOLDER_SHAPE),
                )
                is ReplyQuote.Unavailable -> Text(
                    stringResource(R.string.message_quote_unavailable),
                    style = WontonTheme.typography.supporting,
                    color = secondary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun QuotedContentLine(content: QuotedContent, core: ClientCore, secondary: Color) {
    when (content) {
        is QuotedContent.Text -> Text(
            content.text,
            style = WontonTheme.typography.supporting,
            color = secondary,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        is QuotedContent.Image -> {
            val thumbnail = content.thumbnail
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                if (thumbnail != null) Thumbnail(thumbnail, core, secondary)
                Text(
                    stringResource(R.string.message_quote_image),
                    style = WontonTheme.typography.supporting,
                    color = secondary,
                    maxLines = 1,
                )
            }
        }
        QuotedContent.Redacted -> Text(
            stringResource(R.string.timeline_redacted),
            style = WontonTheme.typography.supporting.copy(fontStyle = FontStyle.Italic),
            color = secondary,
            maxLines = 1,
        )
        QuotedContent.Undecryptable -> Text(
            stringResource(R.string.message_quote_undecryptable),
            style = WontonTheme.typography.supporting,
            color = secondary,
            maxLines = 1,
        )
        QuotedContent.Other -> Text(
            stringResource(R.string.message_quote_other),
            style = WontonTheme.typography.supporting,
            color = secondary,
            maxLines = 1,
        )
    }
}

/** 缩略图解码出来之前先占一块底色，不把这一行撑高或塌下去。 */
@Composable
private fun Thumbnail(thumbnail: MediaHandle, core: ClientCore, secondary: Color) {
    val image by loadedImage(core, thumbnail)
    image?.getOrNull()?.let { bitmap ->
        Image(
            bitmap,
            contentDescription = null,
            modifier = Modifier.size(THUMBNAIL).background(secondary, PLACEHOLDER_SHAPE),
            contentScale = ContentScale.Crop,
        )
    } ?: Box(Modifier.size(THUMBNAIL).background(secondary, PLACEHOLDER_SHAPE))
}

/** 输入框上方的回复状态：引用原消息，带叉可以取消。发送时把事件 ID 交回核心。 */
@Composable
fun ComposerReply(quote: ReplyQuote, core: ClientCore, onCancel: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(
            start = WontonTheme.spacing.s16,
            end = WontonTheme.spacing.s8,
            bottom = WontonTheme.spacing.s4,
        ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        QuotedMessageRow(quote, core, Modifier.weight(1f))
        WontonIconButton(WontonIcon.Close, stringResource(R.string.message_reply_cancel), onCancel)
    }
}

private val THUMBNAIL = 40.dp
private val PLACEHOLDER_SHAPE = RoundedCornerShape(6.dp)
private const val LOADING_FILL = 0.6f
