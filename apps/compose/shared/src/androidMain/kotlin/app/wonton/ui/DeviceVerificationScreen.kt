package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringArrayResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.TabRef
import app.wonton.protocol.VerificationCancel
import app.wonton.protocol.VerificationEmoji
import app.wonton.protocol.VerificationFlow
import app.wonton.protocol.VerificationStage
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** SAS 表情的本地化名字：按 Matrix 规范 SAS 表情表的序号从资源里取（ADR-0046），名字不在代码里写。 */
@Composable
private fun emojiName(index: Int): String {
    val names = stringArrayResource(R.array.verification_emoji)
    return names.getOrElse(index) { "" }
}

/**
 * 一次设备确认进展到哪一步（ADR-0046）。发起的那台设备在聊天备份页显示；来确认的设备在“添加设备”页就地显示，
 * 或由 [DeviceVerificationDialogs] 弹出。确认取消后改用恢复密钥时调 [onUseRecoveryKey]，只有发起确认的设备给。
 */
@Composable
fun VerificationFlowContent(
    flow: VerificationFlow,
    onAction: (Action) -> Unit,
    onUseRecoveryKey: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        when (val stage = flow.stage) {
            VerificationStage.Incoming -> IncomingRequest(flow, onAction)
            VerificationStage.Requested -> Waiting(
                stringResource(R.string.verification_requested),
                stringResource(R.string.verification_requested_hint),
                flow.flowId, onAction,
            )
            VerificationStage.Starting -> Waiting(
                stringResource(R.string.verification_starting), hint = null, flow.flowId, onAction,
            )
            is VerificationStage.Compare -> Compare(stage.emoji, stage.decimals, flow.flowId, onAction)
            VerificationStage.WaitingForOther -> Waiting(
                stringResource(R.string.verification_waiting_for_other), hint = null, flow.flowId, onAction,
            )
            VerificationStage.FetchingKeys -> Waiting(
                stringResource(R.string.verification_fetching_keys), hint = null, flow.flowId, onAction,
            )
            VerificationStage.Done -> Text(
                stringResource(
                    if (flow.startedHere) R.string.verification_done_here else R.string.verification_done_other,
                ),
                style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            is VerificationStage.Cancelled -> Cancelled(flow, stage.reason, onAction, onUseRecoveryKey)
            VerificationStage.Other -> CancelledText(R.string.verification_cancelled_other)
        }
    }
}

@Composable
private fun IncomingRequest(flow: VerificationFlow, onAction: (Action) -> Unit) {
    val name = flow.otherDeviceName?.takeIf { it.isNotEmpty() } ?: flow.otherDeviceId
    Text(
        stringResource(R.string.verification_incoming_message, name),
        style = WontonTheme.typography.itemTitle,
        color = WontonTheme.colors.textPrimary.toColor(),
    )
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        WontonButton(
            stringResource(R.string.verification_accept),
            { onAction(Action.AcceptDeviceVerification(flow.flowId)) },
            Modifier.weight(1f),
        )
        WontonButton(
            stringResource(R.string.verification_deny),
            { onAction(Action.CancelDeviceVerification(flow.flowId)) },
            Modifier.weight(1f),
            WontonButtonKind.SECONDARY,
        )
    }
}

@Composable
private fun Waiting(text: String, hint: String?, flowId: String, onAction: (Action) -> Unit) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.iconSmall),
            color = WontonTheme.colors.textPrimary.toColor(),
            strokeWidth = 2.dp,
        )
        Text(text, style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
    }
    if (hint != null) {
        Text(hint, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
    }
    WontonButton(
        stringResource(R.string.common_cancel),
        { onAction(Action.CancelDeviceVerification(flowId)) },
        Modifier.fillMaxWidth(),
        WontonButtonKind.SECONDARY,
    )
}

@Composable
private fun Compare(
    emoji: List<VerificationEmoji>,
    decimals: List<UShort>,
    flowId: String,
    onAction: (Action) -> Unit,
) {
    Text(
        stringResource(R.string.verification_compare_title),
        style = WontonTheme.typography.itemTitle,
        color = WontonTheme.colors.textPrimary.toColor(),
    )
    CompareBody(emoji, decimals)
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        WontonButton(
            stringResource(R.string.verification_same),
            { onAction(Action.ConfirmVerification(flowId)) },
            Modifier.weight(1f),
        )
        WontonButton(
            stringResource(R.string.verification_different),
            { onAction(Action.DenyVerification(flowId)) },
            Modifier.weight(1f),
            WontonButtonKind.SECONDARY,
        )
    }
    WontonButton(
        stringResource(R.string.common_cancel),
        { onAction(Action.CancelDeviceVerification(flowId)) },
        Modifier.fillMaxWidth(),
        WontonButtonKind.SECONDARY,
    )
}

/** 对照的内容：对端不支持 SAS 表情时，规范回退成三个四位数。 */
@Composable
internal fun CompareBody(emoji: List<VerificationEmoji>, decimals: List<UShort>) {
    if (emoji.isEmpty()) {
        Text(
            decimals.joinToString("  ") { "%04d".format(it.toInt()) },
            style = WontonTheme.typography.body.copy(fontFamily = FontFamily.Monospace),
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        return
    }
    WontonGroup {
        emoji.forEachIndexed { index, item ->
            if (index > 0) WontonSeparator()
            Row(
                Modifier.fillMaxWidth()
                    .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                Text(
                    item.symbol,
                    style = WontonTheme.typography.itemTitle,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
                Text(
                    emojiName(item.index.toInt()),
                    style = WontonTheme.typography.body,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
            }
        }
    }
}

@Composable
private fun Cancelled(
    flow: VerificationFlow,
    reason: VerificationCancel,
    onAction: (Action) -> Unit,
    onUseRecoveryKey: (() -> Unit)?,
) {
    CancelledText(reason.textRes())
    // 下一步只给发起确认的这台设备：“重新开始”是主按钮，恢复密钥是备选。没取全钥匙也先重来：可能只是请求丢了。
    if (!flow.startedHere) return
    when (reason) {
        VerificationCancel.ACCEPTED_ELSEWHERE, VerificationCancel.OTHER -> Unit
        else -> {
            WontonButton(
                stringResource(R.string.verification_retry),
                { onAction(Action.StartDeviceVerification) },
                Modifier.fillMaxWidth(),
            )
            onUseRecoveryKey?.let {
                WontonButton(
                    stringResource(R.string.verification_use_recovery_key), it, Modifier.fillMaxWidth(),
                    WontonButtonKind.SECONDARY,
                )
            }
        }
    }
}

@Composable
private fun CancelledText(textRes: Int) {
    Text(
        stringResource(textRes),
        style = WontonTheme.typography.body,
        color = WontonTheme.colors.textPrimary.toColor(),
    )
}

internal fun VerificationCancel.textRes(): Int = when (this) {
    VerificationCancel.BY_ME -> R.string.verification_cancelled_by_me
    VerificationCancel.BY_OTHER -> R.string.verification_cancelled_by_other
    VerificationCancel.MISMATCH -> R.string.verification_cancelled_mismatch
    VerificationCancel.TIMEOUT -> R.string.verification_cancelled_timeout
    VerificationCancel.ACCEPTED_ELSEWHERE -> R.string.verification_cancelled_accepted_elsewhere
    VerificationCancel.CONCURRENT_REQUEST -> R.string.verification_cancelled_concurrent_request
    VerificationCancel.KEYS_MISSING -> R.string.verification_cancelled_keys_missing
    VerificationCancel.FAILED -> R.string.verification_cancelled_failed
    VerificationCancel.OTHER -> R.string.verification_cancelled_other
}
