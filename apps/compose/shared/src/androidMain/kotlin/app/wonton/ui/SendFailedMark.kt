package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.SendFailure
import app.wonton.protocol.SendState
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon

/** 气泡旁的红色感叹号：点开选重发或删除。讲类别给读屏；核心还分不出类别（`Other`）时讲原文。 */
@Composable
internal fun FailedMark(state: SendState.Failed, onRetry: () -> Unit, onCancel: () -> Unit) {
    var choosing by remember { mutableStateOf(false) }
    val danger = WontonTheme.colors.danger.toColor()
    val text = if (state.recoverable) {
        stringResource(R.string.timeline_send_failed_retrying)
    } else {
        state.details.takeIf { state.failure == SendFailure.OTHER } ?: stringResource(state.failure.text)
    }
    Box(
        Modifier.size(WontonTheme.size.iconSmall).clickable(role = Role.Button) { choosing = true }
            .semantics { contentDescription = text },
        contentAlignment = Alignment.Center,
    ) {
        Text("!", style = WontonTheme.typography.itemTitle, color = danger)
    }
    if (choosing) {
        WontonActionSheet(
            listOf(
                WontonSheetAction(stringResource(R.string.timeline_resend), WontonIcon.Refresh, onClick = onRetry),
                WontonSheetAction(
                    stringResource(R.string.common_delete), WontonIcon.Trash, isDanger = true, onClick = onCancel,
                ),
            ),
            { choosing = false },
            title = state.details,
        )
    }
}

/** 类别由核心给，说法在壳里；英文原文在 [_SendState_Failed.details]，放进面板的标题。 */
internal val SendFailure.text: Int
    get() = when (this) {
        SendFailure.OFFLINE -> R.string.timeline_send_failure_offline
        SendFailure.TOO_LARGE -> R.string.timeline_send_failure_too_large
        SendFailure.UNSUPPORTED_TYPE -> R.string.timeline_send_failure_unsupported_type
        SendFailure.REPLY_TARGET -> R.string.timeline_send_failure_reply_target
        SendFailure.REJECTED -> R.string.timeline_send_failure_rejected
        SendFailure.UPSTREAM -> R.string.timeline_send_failure_upstream
        SendFailure.OTHER -> R.string.timeline_send_failed_other
    }
