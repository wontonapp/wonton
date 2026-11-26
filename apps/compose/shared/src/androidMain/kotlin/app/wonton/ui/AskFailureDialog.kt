package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.AskFailure
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction

/** 问 Bot 失败：一句按原因写的人话，重试带同一个房间；关掉就收起。 */
@Composable
fun AskFailureDialog(failure: AskFailureBanner, onRetry: () -> Unit, onDismiss: () -> Unit) {
    WontonDialog(
        title = failure.reason.message(),
        message = failure.detail.ifEmpty { null },
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_close), onClick = onDismiss),
            WontonDialogAction(stringResource(R.string.common_retry), WontonButtonKind.PRIMARY, onRetry),
        ),
    )
}

@Composable
private fun AskFailure.message(): String = stringResource(
    when (this) {
        AskFailure.NO_BOT -> R.string.chat_ask_failed_no_bot
        AskFailure.BOT_NOT_READY -> R.string.chat_ask_failed_bot_not_ready
        AskFailure.BOT_PROBLEM -> R.string.chat_ask_failed_bot_problem
        AskFailure.NOT_IN_ROOM -> R.string.chat_ask_failed_not_in_room
        AskFailure.READ_ONLY_ROOM -> R.string.chat_ask_failed_read_only_room
        AskFailure.CONTACT_REQUIRED -> R.string.chat_ask_failed_contact_required
        AskFailure.TRANSPORT_UNAVAILABLE -> R.string.chat_ask_failed_transport
        AskFailure.FORBIDDEN -> R.string.chat_ask_failed_forbidden
        AskFailure.NOT_READY -> R.string.chat_ask_failed_not_ready
        AskFailure.UPSTREAM -> R.string.chat_ask_failed_upstream
        AskFailure.OTHER -> R.string.chat_ask_failed_other
    },
)
