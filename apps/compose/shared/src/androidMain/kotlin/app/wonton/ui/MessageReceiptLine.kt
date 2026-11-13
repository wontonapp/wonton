package app.wonton.ui

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.MessageReceipt
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 自己发的最后一条下面那行小字：已送达、已读。别的消息没有；不认识的取值不写。 */
@Composable
internal fun ReceiptLine(receipt: MessageReceipt?) {
    val text = when (receipt) {
        MessageReceipt.DELIVERED -> stringResource(R.string.timeline_delivered)
        MessageReceipt.READ -> stringResource(R.string.timeline_read)
        MessageReceipt.OTHER, null -> return
    }
    Text(
        text,
        Modifier.testTag("messageReceipt"),
        style = WontonTheme.typography.caption,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}
