package app.wonton.ui

import android.content.Context
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PairingFailure
import app.wonton.protocol.PairingStage
import app.wonton.protocol.PairingStatus
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

internal const val PASTED_CODE_LINES = 5

/** 壳的显示门槛（毫秒）：等了这么久还没好，配对页就补“还在连接”和取消；时间本身来自核心的进展。 */
private const val SLOW_AFTER_MS = 5_000uL

/**
 * 扫码配对：扫家里服务器状态页上（或服务器上 `wonton-core pair` 打出）的二维码，也可以粘贴码的文字。
 * 进展和结果只来自核心的 [pairing]（`HomeViewModel.pairing`）；配上后调用 [onPaired]。
 */
@Composable
fun PairingScreen(
    core: ClientCore,
    pairing: PairingStatus?,
    modifier: Modifier = Modifier,
    onPaired: () -> Unit = {},
    onBack: (() -> Unit)? = null,
) {
    val context = LocalContext.current
    var deviceName by rememberSaveable { mutableStateOf(defaultDeviceName(context)) }
    var pastedCode by rememberSaveable { mutableStateOf("") }
    // 核心当场拒绝的原因，比如扫到的不是配对码。
    var refusal by remember { mutableStateOf<Problem?>(null) }
    // 最近一次出示的码。扫到同一个码不再自动出示，失败后由“再试一次”重来。
    var lastCode by rememberSaveable { mutableStateOf<String?>(null) }
    var previous by remember { mutableStateOf(pairing) }
    LaunchedEffect(pairing) {
        // 配上了由核心说：进展从“正在配对”翻到 Paired；取消回到没有进展，不算配上。
        if (previous is PairingStatus.InProgress && pairing is PairingStatus.Paired) onPaired()
        previous = pairing
    }
    val inProgress = pairing is PairingStatus.InProgress
    val focus = LocalFocusManager.current
    fun pair(code: String) {
        if (inProgress) return
        // 收起键盘，让配对结果和操作保持可见。
        focus.clearFocus()
        lastCode = code
        refusal = core.send(Action.PairHome(code, deviceName))
    }
    val leaveFailure = {
        previous = pairing
        focus.clearFocus()
        refusal = null
        core.send(Action.DismissPairingFailure)
        onBack?.invoke()
        Unit
    }
    BackHandler(enabled = pairing is PairingStatus.Failed, onBack = leaveFailure)
    if (pairing is PairingStatus.Failed) {
        PairingFailureNotice(
            pairing,
            onRetry = lastCode?.let { code -> { pair(code) } },
            onBack = leaveFailure,
            modifier = modifier.verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        )
    } else {
        Column(
            modifier.verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            PairingForm(
                pastedCode, { pastedCode = it }, inProgress,
                onScan = { code -> if (code != lastCode) pair(code) }, pair = ::pair,
            )
            refusal?.let { ProblemNotice(it, WontonIcon.Error) }
            if (pairing is PairingStatus.InProgress) {
                PairingProgress(pairing, onCancel = { core.send(Action.CancelPairing) })
            }
            PairingDeviceName(deviceName, onDeviceName = { deviceName = it })
        }
    }
}

@Composable
private fun PairingForm(
    pastedCode: String,
    onPastedCode: (String) -> Unit,
    inProgress: Boolean,
    onScan: (String) -> Unit,
    pair: (String) -> Unit,
) {
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            CameraArea(onCode = onScan)
            Text(
                stringResource(R.string.pairing_instructions),
                style = WontonTheme.typography.supporting,
                color = secondary,
            )
        }
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            val codeLabel = stringResource(R.string.pairing_paste_label)
            Text(
                codeLabel,
                Modifier.semantics { hideFromAccessibility() },
                style = WontonTheme.typography.supporting,
                color = secondary,
            )
            WontonTextField(
                value = pastedCode,
                onValueChange = onPastedCode,
                modifier = Modifier.fillMaxWidth().testTag("pairing-code"),
                placeholder = stringResource(R.string.pairing_paste_placeholder),
                label = codeLabel,
                textStyle = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                maxLines = PASTED_CODE_LINES,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
            )
            WontonButton(
                stringResource(R.string.pairing_submit),
                onClick = { pair(pastedCode) },
                modifier = Modifier.fillMaxWidth().testTag("pairing-submit"),
                enabled = pastedCode.isNotBlank() && !inProgress,
            )
        }
    }
}

@Composable
private fun PairingDeviceName(deviceName: String, onDeviceName: (String) -> Unit) {
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        val deviceNameLabel = stringResource(R.string.pairing_device_name)
        Text(
            deviceNameLabel,
            Modifier.semantics { hideFromAccessibility() },
            style = WontonTheme.typography.supporting,
            color = secondary,
        )
        WontonTextField(
            value = deviceName,
            onValueChange = onDeviceName,
            modifier = Modifier.fillMaxWidth(),
            label = deviceNameLabel,
        )
    }
}

/** 设备列表里这台设备的默认名字：系统设置里的设备名，没有时用型号。 */
fun defaultDeviceName(context: Context): String =
    Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL

/** 配对的进展只反映核心状态；等久了补一句还在哪一步，慢的话可以取消。 */
@Composable
private fun PairingProgress(status: PairingStatus.InProgress, onCancel: () -> Unit) {
    val color = WontonTheme.colors.textPrimary.toColor()
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconSmall), color = color, strokeWidth = 2.dp)
            Text(stringResource(R.string.pairing_in_progress), style = WontonTheme.typography.body, color = color)
        }
        // 壳的显示门槛：等了这么久还没好就补一句还在哪一步和取消；时间本身来自核心的进展。
        if (status.elapsedMs >= SLOW_AFTER_MS) {
            Text(
                stringResource(
                    if (status.stage == PairingStage.CONNECTING) R.string.pairing_still_connecting
                    else R.string.pairing_still_confirming,
                ),
                style = WontonTheme.typography.supporting,
                color = secondary,
            )
            WontonButton(
                stringResource(R.string.common_cancel),
                onCancel,
                modifier = Modifier.fillMaxWidth(),
                kind = WontonButtonKind.SECONDARY,
            )
        }
    }
}

@Composable
private fun PairingFailureNotice(
    status: PairingStatus.Failed,
    onRetry: (() -> Unit)?,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier) {
        WontonNotice(
            WontonIcon.Warning,
            stringResource(failureTitle(status.reason)),
            lines = listOfNotNull(failureHint(status.reason)?.let { stringResource(it) }),
        ) {
            if (onRetry != null && status.reason.canRetrySameCode()) {
                WontonButton(
                    stringResource(R.string.pairing_retry_same_code), onRetry, modifier = Modifier.fillMaxWidth(),
                )
            }
            WontonButton(
                stringResource(R.string.design_back), onBack,
                modifier = Modifier.fillMaxWidth(), kind = WontonButtonKind.SECONDARY,
            )
            if (status.reason.showsCoreMessage()) ProblemDetail(status.message)
        }
    }
}

private fun failureTitle(reason: PairingFailure): Int = when (reason) {
    PairingFailure.REJECTED -> R.string.pairing_failure_rejected
    PairingFailure.EXPIRED -> R.string.pairing_failure_expired
    PairingFailure.CLOSED -> R.string.pairing_failure_closed
    PairingFailure.PENDING_APPROVAL -> R.string.pairing_failure_pending
    PairingFailure.JOIN_REJECTED -> R.string.joining_failure_rejected
    PairingFailure.CODE_USED -> R.string.joining_failure_code_used
    PairingFailure.TRANSPORT_UNAVAILABLE -> R.string.pairing_failure_transport
    PairingFailure.UPSTREAM -> R.string.problem_upstream_title
    PairingFailure.PROTOCOL -> R.string.pairing_failure_protocol
}

/** 码被拒、过期、停用这几类，服务器写的说明和标题说的是同一件事，只留标题和下一步。等批的说明是核心写的，带加入后的角色。 */
private fun failureHint(reason: PairingFailure): Int? = when (reason) {
    PairingFailure.REJECTED, PairingFailure.EXPIRED -> R.string.pairing_next_new_code
    PairingFailure.CLOSED -> R.string.pairing_next_closed
    PairingFailure.PENDING_APPROVAL -> R.string.pairing_next_pending
    PairingFailure.JOIN_REJECTED, PairingFailure.CODE_USED -> R.string.joining_next_invite
    PairingFailure.TRANSPORT_UNAVAILABLE -> R.string.pairing_hint_transport
    PairingFailure.PROTOCOL -> R.string.pairing_hint_protocol
    PairingFailure.UPSTREAM -> null
}

/** 核心按 App 语言生成的诊断信息只在默认收起的详情里显示。 */
private fun PairingFailure.showsCoreMessage(): Boolean = when (this) {
    PairingFailure.TRANSPORT_UNAVAILABLE, PairingFailure.UPSTREAM, PairingFailure.PROTOCOL,
    PairingFailure.PENDING_APPROVAL -> true
    PairingFailure.REJECTED, PairingFailure.EXPIRED, PairingFailure.CLOSED -> false
    PairingFailure.JOIN_REJECTED, PairingFailure.CODE_USED -> false
}

/** 连不上和服务器出错可能是暂时的，同一个码可以再试；等批的在同意后重试同一个码。其余原因要换码或者更新。 */
private fun PairingFailure.canRetrySameCode() =
    this == PairingFailure.TRANSPORT_UNAVAILABLE || this == PairingFailure.UPSTREAM ||
        this == PairingFailure.PENDING_APPROVAL
