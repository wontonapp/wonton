package app.wonton.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.CameraOff
import app.wonton.ui.icons.WontonIcon

private val SCANNER_HEIGHT = 280.dp

/**
 * 扫码的取景框和相机权限：没问过就问；拒绝过一次还能再问；选了“不再询问”以后只能去系统设置里开。
 * 相机中途出错时说原因，按“重试”重新开。扫到的码交给 [onCode]，同一个码只报一次。
 */
@Composable
internal fun CameraArea(onCode: (String) -> Unit) {
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
                WontonButton(
                    stringResource(R.string.common_retry), { problem = null }, kind = WontonButtonKind.SECONDARY,
                )
            }
        } ?: QrScanner(
            onCode,
            onUnavailable = { problem = it },
            Modifier.fillMaxWidth()
                .height(SCANNER_HEIGHT)
                .clip(RoundedCornerShape(WontonTheme.radii.card)),
        )
        CameraAccess.Asking -> Text(
            stringResource(R.string.camera_waiting),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        CameraAccess.Denied -> WontonNotice(WontonIcon.CameraOff, stringResource(R.string.camera_denied)) {
            WontonButton(
                stringResource(R.string.camera_allow),
                { request.launch(Manifest.permission.CAMERA) },
                kind = WontonButtonKind.SECONDARY,
            )
        }
        CameraAccess.Blocked -> WontonNotice(WontonIcon.CameraOff, stringResource(R.string.camera_blocked)) {
            WontonButton(
                stringResource(R.string.camera_settings),
                { context.startActivity(appSettings(context)) },
                kind = WontonButtonKind.SECONDARY,
            )
        }
        CameraAccess.Missing -> WontonNotice(
            WontonIcon.CameraOff,
            stringResource(R.string.camera_missing),
            isError = false,
        )
    }
}

internal enum class CameraAccess { Granted, Asking, Denied, Blocked, Missing }

internal fun cameraAccess(context: Context): CameraAccess = when {
    !context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) -> CameraAccess.Missing
    ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED ->
        CameraAccess.Granted
    else -> CameraAccess.Asking
}

private fun appSettings(context: Context) =
    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
