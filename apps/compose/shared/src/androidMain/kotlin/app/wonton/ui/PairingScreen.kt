package app.wonton.ui

import android.content.Context
import android.os.Build
import android.provider.Settings
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PairingFailure
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
) {
    val context = LocalContext.current
    var deviceName by rememberSaveable { mutableStateOf(defaultDeviceName(context)) }
    var pastedCode by rememberSaveable { mutableStateOf("") }
    // 核心当场拒绝的原因，比如扫到的不是配对码。
    var refusal by remember { mutableStateOf<Problem?>(null) }
    // 最近一次出示的码。扫到同一个码不再自动出示，失败后由“再试一次”重来。
    var lastCode by remember { mutableStateOf<String?>(null) }
    var previous by remember { mutableStateOf(pairing) }
    LaunchedEffect(pairing) {
        if (previous is PairingStatus.InProgress && pairing == null) onPaired()
        previous = pairing
    }
    val inProgress = pairing is PairingStatus.InProgress
    val focus = LocalFocusManager.current
    fun pair(code: String) {
        if (inProgress) return
        // 结果显示在按钮下面，键盘不收会挡住。
        focus.clearFocus()
        lastCode = code
        refusal = core.send(Action.PairHome(code, deviceName))
    }
    val secondary = WontonTheme.colors.textSecondary.toColor()
    Column(
        modifier.verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            CameraArea(onCode = { code -> if (code != lastCode) pair(code) })
            Text(
                stringResource(R.string.pairing_instructions),
                style = WontonTheme.typography.supporting,
                color = secondary,
            )
        }
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonTextField(
                value = pastedCode,
                onValueChange = { pastedCode = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.pairing_paste_placeholder),
                textStyle = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                maxLines = PASTED_CODE_LINES,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
            )
            WontonButton(
                stringResource(R.string.pairing_submit),
                onClick = { pair(pastedCode) },
                modifier = Modifier.fillMaxWidth(),
                enabled = pastedCode.isNotBlank() && !inProgress,
            )
        }
        refusal?.let { ProblemNotice(it, WontonIcon.Error) }
        pairing?.let { status -> PairingProgress(status, onRetry = lastCode?.let { code -> { pair(code) } }) }
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
                onValueChange = { deviceName = it },
                modifier = Modifier.fillMaxWidth(),
                label = deviceNameLabel,
            )
        }
    }
}

/** 设备列表里这台设备的默认名字：系统设置里的设备名，没有时用型号。 */
fun defaultDeviceName(context: Context): String =
    Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL

/** 配对的进展；失败时按原因说下一步怎么做。 */
@Composable
private fun PairingProgress(status: PairingStatus, onRetry: (() -> Unit)?) {
    when (status) {
        is PairingStatus.InProgress -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            val color = WontonTheme.colors.textPrimary.toColor()
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconSmall), color = color, strokeWidth = 2.dp)
            Text(stringResource(R.string.pairing_in_progress), style = WontonTheme.typography.body, color = color)
        }
        is PairingStatus.Failed -> WontonNotice(
            WontonIcon.Warning,
            stringResource(failureTitle(status.reason)),
            lines = listOfNotNull(failureHint(status.reason)?.let { stringResource(it) }),
        ) {
            if (status.reason.showsCoreMessage()) ProblemDetail(status.message)
            if (onRetry != null && status.reason.canRetrySameCode()) {
                WontonButton(
                    stringResource(R.string.pairing_retry_same_code), onRetry, kind = WontonButtonKind.SECONDARY,
                )
            }
        }
    }
}

private fun failureTitle(reason: PairingFailure): Int = when (reason) {
    PairingFailure.REJECTED -> R.string.pairing_failure_rejected
    PairingFailure.EXPIRED -> R.string.pairing_failure_expired
    PairingFailure.CLOSED -> R.string.pairing_failure_closed
    PairingFailure.TRANSPORT_UNAVAILABLE -> R.string.problem_transport_unavailable_title
    PairingFailure.UPSTREAM -> R.string.problem_upstream_title
    PairingFailure.PROTOCOL -> R.string.pairing_failure_protocol
}

/** 码被拒、过期、停用这几类，服务器写的说明和标题说的是同一件事，只留标题和下一步。 */
private fun failureHint(reason: PairingFailure): Int? = when (reason) {
    PairingFailure.REJECTED, PairingFailure.EXPIRED -> R.string.pairing_next_new_code
    PairingFailure.CLOSED -> R.string.pairing_next_closed
    PairingFailure.TRANSPORT_UNAVAILABLE -> R.string.pairing_hint_transport
    PairingFailure.PROTOCOL -> R.string.pairing_hint_protocol
    PairingFailure.UPSTREAM -> null
}

/** 服务器给的原因是原始值（语言是服务器主人的），收进默认收起的详情。 */
private fun PairingFailure.showsCoreMessage(): Boolean = when (this) {
    PairingFailure.TRANSPORT_UNAVAILABLE, PairingFailure.UPSTREAM, PairingFailure.PROTOCOL -> true
    PairingFailure.REJECTED, PairingFailure.EXPIRED, PairingFailure.CLOSED -> false
}

/** 连不上和服务器出错可能是暂时的，同一个码可以再试；其余原因要换码或者更新。 */
private fun PairingFailure.canRetrySameCode() =
    this == PairingFailure.TRANSPORT_UNAVAILABLE || this == PairingFailure.UPSTREAM
