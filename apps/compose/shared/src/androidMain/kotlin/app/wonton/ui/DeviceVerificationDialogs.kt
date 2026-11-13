package app.wonton.ui

import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.TabRef
import app.wonton.protocol.VerificationFlow
import app.wonton.protocol.VerificationStage
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 来确认的设备收到的确认（ADR-0046）：一个弹窗跟着流程走，请求问“开始确认/拒绝”，对照出“相同/不同”。
 * “添加设备”和聊天备份页这两处不管什么状态都在页面里显示流程（[InviteScreen]、[ChatBackupScreen]），它们开着时
 * 弹窗不出现：页面显示和这里压下用的是同一条判断，页面改了要一起改。
 * 进行中收起弹窗不取消确认，确认还在继续；结果收起后就不再弹，这两个页面里还看得到。
 */
@Composable
internal fun DeviceVerificationDialogs(chat: ChatViewModel, navigation: AppNavigation, core: ClientCore) {
    val flow = chat.encryption.verificationFlow
    var dismissedFor by remember { mutableStateOf<String?>(null) }
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val onAction: (Action) -> Unit = { action -> rejection = core.send(action) }
    if (dialogDue(flow, dismissedFor, navigation)) {
        flow?.let { VerificationDialog(it, onAction, onDismiss = { dismissedFor = it.flowId }) }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

/** 请求要主人点按钮回答，收起过、别处正显示、或不是来确认的设备时都不弹。 */
private fun dialogDue(flow: VerificationFlow?, dismissedFor: String?, navigation: AppNavigation): Boolean {
    if (flow == null || flow.startedHere || dismissedFor == flow.flowId) return false
    return !surfaceShowing(navigation)
}

@Composable
private fun VerificationDialog(flow: VerificationFlow, onAction: (Action) -> Unit, onDismiss: () -> Unit) {
    val title = stringResource(R.string.verification_title)
    when (val stage = flow.stage) {
        VerificationStage.Incoming -> {
            val name = flow.otherDeviceName?.takeIf { it.isNotEmpty() } ?: flow.otherDeviceId
            // 请求必须由主人点按钮回答，点空白处不算回答，弹窗不走。
            WontonDialog(
                title,
                stringResource(R.string.verification_incoming_message, name),
                onDismiss = {},
                actions = listOf(
                    WontonDialogAction(stringResource(R.string.verification_accept)) {
                        onAction(Action.AcceptDeviceVerification(flow.flowId))
                    },
                    WontonDialogAction(stringResource(R.string.verification_deny)) {
                        onAction(Action.CancelDeviceVerification(flow.flowId))
                    },
                ),
            )
        }
        VerificationStage.Requested, VerificationStage.Starting, VerificationStage.WaitingForOther,
        VerificationStage.FetchingKeys,
        -> WontonDialog(
            title,
            stringResource(stage.waitingTextRes()),
            onDismiss,
            listOf(cancelAction(flow, onAction)),
        ) {
            CircularProgressIndicator(
                Modifier.size(WontonTheme.size.iconSmall),
                color = WontonTheme.colors.textPrimary.toColor(),
                strokeWidth = 2.dp,
            )
        }
        is VerificationStage.Compare -> WontonDialog(
            stringResource(R.string.verification_compare_title),
            null,
            onDismiss,
            listOf(
                WontonDialogAction(stringResource(R.string.verification_same), WontonButtonKind.PRIMARY) {
                    onAction(Action.ConfirmVerification(flow.flowId))
                },
                WontonDialogAction(stringResource(R.string.verification_different)) {
                    onAction(Action.DenyVerification(flow.flowId))
                },
                cancelAction(flow, onAction),
            ),
        ) { CompareBody(stage.emoji, stage.decimals) }
        VerificationStage.Done -> WontonDialog(
            title,
            stringResource(
                if (flow.startedHere) R.string.verification_done_here else R.string.verification_done_other,
            ),
            onDismiss,
            listOf(okAction(onDismiss)),
        )
        is VerificationStage.Cancelled, VerificationStage.Other -> WontonDialog(
            title,
            stringResource(cancelledTextRes(stage)),
            onDismiss,
            listOf(okAction(onDismiss)),
        )
    }
}

private fun cancelledTextRes(stage: VerificationStage): Int =
    if (stage is VerificationStage.Cancelled) stage.reason.textRes() else R.string.verification_cancelled_other

private fun VerificationStage.waitingTextRes(): Int = when (this) {
    VerificationStage.Requested -> R.string.verification_requested
    VerificationStage.Starting -> R.string.verification_starting
    VerificationStage.WaitingForOther -> R.string.verification_waiting_for_other
    VerificationStage.FetchingKeys -> R.string.verification_fetching_keys
    else -> error("Not a waiting stage: $this")
}

@Composable
private fun cancelAction(flow: VerificationFlow, onAction: (Action) -> Unit) = WontonDialogAction(
    stringResource(R.string.common_cancel),
) { onAction(Action.CancelDeviceVerification(flow.flowId)) }

@Composable
private fun okAction(onDismiss: () -> Unit) = WontonDialogAction(
    stringResource(R.string.common_ok),
    onClick = onDismiss,
)

/** “添加设备”或聊天备份页正开着：确认流程在这两处就地显示（ADR-0046），不弹窗。 */
private fun surfaceShowing(navigation: AppNavigation): Boolean {
    // 页面盖着时确认流程进不去那两层：设置里看邀请和备份，聊天里看备份。
    val settingsVisible = navigation.settingsOverlay || navigation.appsPage != null
    return (settingsVisible && navigation.settingsPage == SettingsSubpage.INVITE) ||
        (navigation.chat.showingBackup) ||
        (settingsVisible && navigation.settingsPage == SettingsSubpage.BACKUP)
}
