package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.OutgoingMessage
import app.wonton.protocol.SendState
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WarningCircle
import app.wonton.ui.icons.WontonIcon
import java.text.NumberFormat

/** 一行最多并排几个附件。 */
private const val TILES_PER_ROW = 3

/**
 * 点了发送、还没交给 Matrix 的一条（ADR-0094），画在时间线最新一条下面。附件各自画着上传的进展；交出去以后由时间线里的
 * 本地回显代替。传着的时候点状态能取消，传不上去时整条变红，点开重发或者删掉。
 */
@Composable
internal fun OutgoingRow(message: OutgoingMessage, roomId: String, core: ClientCore, onAction: (Action) -> Unit) {
    val colors = WontonTheme.colors
    Row(Modifier.fillMaxWidth().testTag("outgoingMessage")) {
        Spacer(Modifier.weight(1f).widthIn(min = WontonTheme.spacing.s48))
        Column(
            horizontalAlignment = Alignment.End,
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            if (message.attachments.isNotEmpty()) {
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4, Alignment.End),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
                    maxItemsInEachRow = TILES_PER_ROW,
                ) {
                    message.attachments.forEach { attachment ->
                        AttachmentTile(attachment, roomId, core, onAction)
                    }
                }
            }
            if (message.body.isNotEmpty()) {
                Text(
                    message.body,
                    Modifier.accentFill(RoundedCornerShape(WontonTheme.radii.bubble))
                        .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8),
                    style = WontonTheme.typography.body,
                    color = colors.textOnAccentFill.toColor(),
                )
            }
            OutgoingFooter(message, roomId, onAction)
        }
    }
}

@Composable
private fun OutgoingFooter(message: OutgoingMessage, roomId: String, onAction: (Action) -> Unit) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.caption
    var choosing by remember { mutableStateOf(false) }
    val target = TimelineTarget(roomId, null)
    val cancel = { onAction(Action.Timeline(TimelineAction.CancelOutgoing(target, message.id))) }
    Row(
        Modifier.clickable(role = Role.Button) { choosing = true },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        when (val state = message.state) {
            is SendState.Sending -> Text(
                state.progress?.let { progress ->
                    val percent = NumberFormat.getPercentInstance().format(progress.toDouble())
                    stringResource(R.string.timeline_sending_progress, percent)
                } ?: stringResource(R.string.timeline_sending),
                style = style,
                color = colors.textSecondary.toColor(),
            )
            is SendState.Failed -> {
                val danger = colors.danger.toColor()
                Icon(
                    WontonIcon.WarningCircle,
                    // 装饰：失败原因写在右边的文字里。
                    contentDescription = null,
                    modifier = Modifier.size(WontonTheme.size.iconTiny),
                    tint = danger,
                )
                Text(stringResource(R.string.timeline_send_failed, state.message), style = style, color = danger)
            }
            SendState.Sent -> Unit
        }
    }
    if (!choosing) return
    val actions = when (message.state) {
        // 传着的时候也能取消：整条不发，上传停下（盲测：取消了照样发出去）。
        is SendState.Sending -> listOf(
            WontonSheetAction(
                stringResource(R.string.timeline_cancel_send), WontonIcon.Close, isDanger = true, onClick = cancel,
            ),
        )
        is SendState.Failed -> listOf(
            WontonSheetAction(stringResource(R.string.timeline_resend), WontonIcon.Refresh) {
                onAction(Action.Timeline(TimelineAction.RetryOutgoing(target, message.id)))
            },
            WontonSheetAction(
                stringResource(R.string.common_delete), WontonIcon.Trash, isDanger = true, onClick = cancel,
            ),
        )
        SendState.Sent -> emptyList()
    }
    if (actions.isEmpty()) {
        choosing = false
    } else {
        WontonActionSheet(actions, { choosing = false })
    }
}
