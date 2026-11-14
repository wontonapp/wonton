package app.wonton.ui

import android.content.Context
import android.os.Build
import android.provider.Settings
import android.view.HapticFeedbackConstants
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
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.HomeAction
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
    var pastedCode by remember { mutableStateOf("") }
    // 核心当场拒绝的原因，比如扫到的不是配对码。
    var refusal by remember { mutableStateOf<Problem?>(null) }
    var lastCode by remember { mutableStateOf<String?>(null) }
    var scanRecognized by remember { mutableStateOf(false) }
    var status by remember(pairing) { mutableStateOf(core.home().pairing) }
    var submitted by remember { mutableStateOf(status is PairingStatus.InProgress) }
    LaunchedEffect(status, submitted) {
        if (submitted && status is PairingStatus.Paired) {
            submitted = false
            onPaired()
        }
    }
    val view = LocalView.current
    val focus = LocalFocusManager.current
    fun pair(code: String) {
        if (core.home().pairing is PairingStatus.InProgress) return
        // 收起键盘，让配对结果和操作保持可见。
        focus.clearFocus()
        lastCode = code
        refusal = core.send(Action.Home(HomeAction.PairHome(code, deviceName)))?.pairingRefusal()
        submitted = refusal == null
        status = core.home().pairing
    }
    val scanAgain = {
        focus.clearFocus()
        refusal = null
        lastCode = null
        pastedCode = ""
        scanRecognized = false
        submitted = false
        core.send(Action.Home(HomeAction.DismissPairingFailure))
        status = core.home().pairing
        Unit
    }
    val leaveFailure = {
        scanAgain()
        onBack?.invoke()
        Unit
    }
    BackHandler(enabled = status is PairingStatus.Failed, onBack = leaveFailure)
    val failed = status as? PairingStatus.Failed
    if (failed != null) {
        PairingFailureNotice(
            failed,
            onRetry = lastCode?.let { code -> { pair(code) } },
            onScanAgain = scanAgain,
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
            val progress = status as? PairingStatus.InProgress
            if (progress != null) {
                PairingProgress(progress, scanRecognized, onCancel = {
                    core.send(Action.Home(HomeAction.CancelPairing))
                    submitted = false
                    status = core.home().pairing
                    scanRecognized = false
                })
            } else if (refusal != null) {
                ProblemNotice(requireNotNull(refusal), WontonIcon.Error)
                WontonButton(stringResource(R.string.pairing_scan_again), scanAgain)
            } else {
                PairingForm(pastedCode, { pastedCode = it }, onScan = { code ->
                    if (!scanRecognized && core.home().pairing !is PairingStatus.InProgress) {
                        scanRecognized = true
                        view.performHapticFeedback(
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) HapticFeedbackConstants.CONFIRM
                            else HapticFeedbackConstants.VIRTUAL_KEY,
                        )
                        pair(code)
                    }
                }, pair = ::pair)
                PairingDeviceName(deviceName, onDeviceName = { deviceName = it })
            }
        }
    }
}

@Composable
private fun PairingForm(
    pastedCode: String,
    onPastedCode: (String) -> Unit,
    onScan: (String) -> Unit,
    pair: (String) -> Unit,
) {
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            CameraArea(onCode = onScan, singleScan = true)
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
                enabled = pastedCode.isNotBlank(),
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
private fun PairingProgress(status: PairingStatus.InProgress, scanRecognized: Boolean, onCancel: () -> Unit) {
    val color = WontonTheme.colors.textPrimary.toColor()
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconSmall), color = color, strokeWidth = 2.dp)
            Text(
                stringResource(if (scanRecognized) R.string.pairing_scan_recognized else R.string.pairing_in_progress),
                style = WontonTheme.typography.body,
                color = color,
            )
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
fun PairingFailureNotice(
    status: PairingStatus.Failed,
    onRetry: (() -> Unit)?,
    onScanAgain: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier) {
        WontonNotice(
            WontonIcon.Warning,
            stringResource(failureTitle(status.reason)),
            lines = listOfNotNull(failureHint(status.reason)?.let { stringResource(it) }),
        ) {
            WontonButton(
                stringResource(R.string.pairing_scan_again), onScanAgain,
                modifier = Modifier.fillMaxWidth().testTag("pairing-scan-again"),
            )
            if (onRetry != null && status.reason.canRetrySameCode()) {
                WontonButton(
                    stringResource(R.string.pairing_retry_same_code), onRetry,
                    modifier = Modifier.fillMaxWidth().testTag("pairing-retry"),
                )
            }
            WontonButton(
                stringResource(R.string.design_back), onBack,
                modifier = Modifier.fillMaxWidth().testTag("pairing-back"), kind = WontonButtonKind.SECONDARY,
            )
            if (status.message.isNotBlank()) ProblemDetail(status.message)
            status.diagnostic?.let { ProblemDetail(it, stringResource(R.string.common_diagnostics)) }
        }
    }
}

private fun failureTitle(reason: PairingFailure): Int = when (reason) {
    PairingFailure.REJECTED -> R.string.pairing_failure_rejected
    PairingFailure.EXPIRED -> R.string.pairing_failure_expired
    PairingFailure.USED -> R.string.pairing_scan_used
    PairingFailure.CLOSED -> R.string.pairing_failure_closed
    PairingFailure.PENDING_APPROVAL -> R.string.pairing_failure_pending
    PairingFailure.JOIN_REJECTED -> R.string.joining_failure_rejected
    PairingFailure.CODE_USED -> R.string.joining_failure_code_used
    PairingFailure.TRANSPORT_UNAVAILABLE -> R.string.pairing_failure_transport
    PairingFailure.UPSTREAM -> R.string.problem_upstream_title
    PairingFailure.PROTOCOL -> R.string.pairing_failure_protocol
}

private fun failureHint(reason: PairingFailure): Int? = when (reason) {
    PairingFailure.REJECTED, PairingFailure.EXPIRED, PairingFailure.USED -> R.string.pairing_next_new_code
    PairingFailure.CLOSED -> R.string.pairing_next_closed
    PairingFailure.PENDING_APPROVAL -> R.string.pairing_next_pending
    PairingFailure.JOIN_REJECTED, PairingFailure.CODE_USED -> R.string.joining_next_invite
    PairingFailure.TRANSPORT_UNAVAILABLE -> R.string.pairing_hint_transport
    PairingFailure.PROTOCOL -> R.string.pairing_hint_protocol
    PairingFailure.UPSTREAM -> null
}
/** 连不上和服务器出错可能是暂时的，同一个码可以再试；等批的在同意后重试同一个码。其余原因要换码或者更新。 */
private fun PairingFailure.canRetrySameCode() =
    this == PairingFailure.TRANSPORT_UNAVAILABLE || this == PairingFailure.UPSTREAM ||
        this == PairingFailure.PENDING_APPROVAL

internal fun Problem.pairingRefusal(): Problem = copy(
    nextStep = null,
    detail = listOfNotNull(nextStep, detail).filter { it.isNotBlank() }.joinToString("\n\n")
        .takeIf { it.isNotBlank() },
)
