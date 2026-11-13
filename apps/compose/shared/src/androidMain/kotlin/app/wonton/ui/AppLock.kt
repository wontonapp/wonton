package app.wonton.ui

import android.content.Context
import android.content.Intent
import android.provider.Settings
import android.view.WindowManager
import androidx.activity.compose.LocalActivity
import androidx.biometric.BiometricManager
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.client.ClientCore
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Lock
import app.wonton.ui.icons.WontonIcon

/**
 * App 锁的遮罩（ADR-0102）：锁定期间盖住整个 App，锁开着的画面再过一遍。
 * 遮罩要在根上，挡住深链进来的页面、设置页和插件网页。
 */
@Composable
fun AppLockGate(core: ClientCore, content: @Composable () -> Unit) {
    val view by remember(core) { core.appLockUpdates() }.collectAsState(remember(core) { core.appLock() })
    // 开着就防最近任务和截屏（先例见 SecretInputs）：锁定判定是核心的事，这里只看开关。
    SecureWhileEnabled(view.enabled)
    val activity = LocalActivity.current as? FragmentActivity
    val auth = remember(activity) { activity?.let(::SystemAuth) }
    // 设备后来没了锁屏（主人关了系统密码）时验不过去：停在锁页，指去系统设置。
    val canAuthenticate =
        remember(view.locked, activity) {
            auth?.canAuthenticate() != BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED
        }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val context = LocalContext.current
    val promptTitle = stringResource(R.string.applock_locked_title)
    val promptReason = stringResource(R.string.applock_verify_reason)
    Box(Modifier.fillMaxSize()) {
        content()
        if (view.locked) {
            AppLockScreen(
                needsDeviceLock = !canAuthenticate,
                onUnlock = { auth?.prompt(promptTitle, promptReason) { passed -> if (passed == AuthResult.Passed) core.unlockApp() } },
                onOpenSettings = { context.openSecuritySettings() },
            )
        }
    }
    // 页面看得见才弹验证：在后台弹出的会被系统收走，取消后停在锁页可再点。
    DisposableEffect(lifecycle, view.locked, auth) {
        if (auth == null || !view.locked) return@DisposableEffect onDispose {}
        val prompt = {
            auth.prompt(promptTitle, promptReason) { result ->
                if (result == AuthResult.Passed) core.unlockApp()
            }
        }
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) prompt()
        }
        lifecycle.addObserver(observer)
        if (lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) prompt()
        onDispose { lifecycle.removeObserver(observer) }
    }
}

/** 锁开着时整段装到当前 Activity 的窗口上：截屏和最近任务缩略图看不到内容。 */
@Composable
private fun SecureWhileEnabled(enabled: Boolean) {
    val activity = LocalActivity.current ?: return
    DisposableEffect(activity, enabled) {
        if (enabled) activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        onDispose { if (enabled) activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
    }
}

@Composable
private fun AppLockScreen(needsDeviceLock: Boolean, onUnlock: () -> Unit, onOpenSettings: () -> Unit) {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize()
            .background(colors.canvas.toColor())
            .padding(WontonTheme.spacing.pageInset),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(
            WontonIcon.Lock,
            contentDescription = null,
            Modifier.size(WontonTheme.size.detailIcon),
            tint = colors.accentInk.toColor(),
        )
        Spacer(Modifier.height(WontonTheme.spacing.s16))
        Text(
            stringResource(R.string.applock_locked_title),
            style = WontonTheme.typography.screenTitle,
            color = colors.textPrimary.toColor(),
        )
        if (needsDeviceLock) {
            Spacer(Modifier.height(WontonTheme.spacing.s8))
            Text(
                stringResource(R.string.applock_no_device_lock),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(WontonTheme.spacing.s16))
            WontonButton(stringResource(R.string.applock_open_settings), onOpenSettings)
        } else {
            Spacer(Modifier.height(WontonTheme.spacing.s16))
            WontonButton(stringResource(R.string.applock_unlock), onUnlock)
        }
    }
}

private fun Context.openSecuritySettings() {
    startActivity(Intent(Settings.ACTION_SECURITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}
