package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.sizeIn
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

/** 气泡旁的红色感叹号：点开选重发或删除。讲类别给读屏；核心还分不出类别（`Other`）时讲原文。命中区至少 48dp（ADR-0026）。 */
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
        Modifier
            .sizeIn(minWidth = WontonTheme.size.target, minHeight = WontonTheme.size.target)
            .clickable(role = Role.Button) { choosing = true }
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
