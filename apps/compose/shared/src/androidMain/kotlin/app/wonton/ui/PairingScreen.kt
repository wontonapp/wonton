package app.wonton.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
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
import app.wonton.ui.icons.CameraOff
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

private const val PASTED_CODE_LINES = 5
private val SCANNER_HEIGHT = 280.dp

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
                "在家里的局域网里，用浏览器打开服务器的 IP 地址（装好时终端里给出），页面上有配对二维码。",
                style = WontonTheme.typography.supporting,
                color = secondary,
            )
        }
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonTextField(
                value = pastedCode,
                onValueChange = { pastedCode = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = "粘贴配对码",
                textStyle = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                maxLines = PASTED_CODE_LINES,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
            )
            WontonButton(
                "配对",
                onClick = { pair(pastedCode) },
                modifier = Modifier.fillMaxWidth(),
                enabled = pastedCode.isNotBlank() && !inProgress,
            )
        }
        refusal?.let { ProblemNotice(it, WontonIcon.Error) }
        pairing?.let { status -> PairingProgress(status, onRetry = lastCode?.let { code -> { pair(code) } }) }
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            Text(
                "这台设备的名字",
                Modifier.semantics { hideFromAccessibility() },
                style = WontonTheme.typography.supporting,
                color = secondary,
            )
            WontonTextField(
                value = deviceName,
                onValueChange = { deviceName = it },
                modifier = Modifier.fillMaxWidth(),
                label = "这台设备的名字",
            )
        }
    }
}

/** 设备列表里这台设备的默认名字：系统设置里的设备名，没有时用型号。 */
fun defaultDeviceName(context: Context): String =
    Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL

private enum class CameraAccess { Granted, Asking, Denied, Blocked, Missing }

/** 取景框和相机权限：没问过就问；拒绝过一次还能再问；选了“不再询问”以后只能去系统设置里开。 */
@Composable
private fun CameraArea(onCode: (String) -> Unit) {
    val context = LocalContext.current
    val activity = LocalActivity.current
    var access by remember { mutableStateOf(cameraAccess(context)) }
    var problem by remember { mutableStateOf<String?>(null) }
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val canAskAgain = activity != null &&
            ActivityCompat.shouldShowRequestPermissionRationale(activity, Manifest.permission.CAMERA)
        access = when {
            granted -> CameraAccess.Granted
            canAskAgain -> CameraAccess.Denied
            else -> CameraAccess.Blocked
        }
    }
    LaunchedEffect(Unit) {
        if (access == CameraAccess.Asking) request.launch(Manifest.permission.CAMERA)
    }
    // 可能刚从系统设置里开了权限回来。
    LifecycleResumeEffect(Unit) {
        if (cameraAccess(context) == CameraAccess.Granted) access = CameraAccess.Granted
        onPauseOrDispose {}
    }
    when (access) {
        CameraAccess.Granted -> problem?.let { reason ->
            WontonNotice(WontonIcon.CameraOff, reason) {
                WontonButton("重新打开相机", { problem = null }, kind = WontonButtonKind.SECONDARY)
            }
        } ?: QrScanner(
            onCode,
            onUnavailable = { problem = it },
            Modifier.fillMaxWidth()
                .height(SCANNER_HEIGHT)
                .clip(RoundedCornerShape(WontonTheme.radii.card)),
        )
        CameraAccess.Asking -> Text(
            "等待相机权限…",
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        CameraAccess.Denied -> WontonNotice(WontonIcon.CameraOff, "云吞没有相机权限，扫不了码。可以允许使用相机，或者粘贴配对码。") {
            WontonButton(
                "允许使用相机",
                { request.launch(Manifest.permission.CAMERA) },
                kind = WontonButtonKind.SECONDARY,
            )
        }
        CameraAccess.Blocked -> WontonNotice(WontonIcon.CameraOff, "云吞没有相机权限，扫不了码。可以在系统设置里打开，或者粘贴配对码。") {
            WontonButton(
                "打开系统设置",
                { context.startActivity(appSettings(context)) },
                kind = WontonButtonKind.SECONDARY,
            )
        }
        CameraAccess.Missing -> WontonNotice(WontonIcon.CameraOff, "这台设备没有相机，请粘贴配对码。", isError = false)
    }
}

private fun cameraAccess(context: Context): CameraAccess = when {
    !context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) -> CameraAccess.Missing
    ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED ->
        CameraAccess.Granted
    else -> CameraAccess.Asking
}

private fun appSettings(context: Context) =
    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))

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
            Text("正在配对…", style = WontonTheme.typography.body, color = color)
        }
        is PairingStatus.Failed -> WontonNotice(
            WontonIcon.Warning,
            failureTitle(status.reason),
            lines = listOfNotNull(status.message, failureHint(status.reason)),
        ) {
            if (onRetry != null && status.reason.canRetrySameCode()) {
                WontonButton("用这个码再试一次", onRetry, kind = WontonButtonKind.SECONDARY)
            }
        }
    }
}

private fun failureTitle(reason: PairingFailure): String = when (reason) {
    PairingFailure.REJECTED -> "服务器不认这个配对码"
    PairingFailure.EXPIRED -> "配对码过期了"
    PairingFailure.CLOSED -> "这个配对码已经停用"
    PairingFailure.TRANSPORT_UNAVAILABLE -> "连不上家里的服务器"
    PairingFailure.UPSTREAM -> "家里的服务器出错了"
    PairingFailure.PROTOCOL -> "App 和服务器的版本对不上"
}

/** 服务器拒绝时，它的说明里已经写了下一步；本机这一侧的失败补一句。 */
private fun failureHint(reason: PairingFailure): String? = when (reason) {
    PairingFailure.TRANSPORT_UNAVAILABLE -> "请确认手机和服务器在同一个局域网里，或者服务器连得上互联网，然后再试一次。"
    PairingFailure.PROTOCOL -> "请把 App 和服务器都更新到最新版本。"
    else -> null
}

/** 连不上和服务器出错可能是暂时的，同一个码可以再试；其余原因要换码或者更新。 */
private fun PairingFailure.canRetrySameCode() =
    this == PairingFailure.TRANSPORT_UNAVAILABLE || this == PairingFailure.UPSTREAM
