package app.wonton.ui

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * 通知权限（Android 13 起要在运行时申请）：查状态和申请都走这一处，推送登记和“映射到本机”共用一套。
 * 要在 Activity 的 `onCreate` 里、界面挂上之前创建：申请的回调只能在创建时登记。
 */
class NotificationPermission(private val activity: ComponentActivity) {
    private val callbacks = mutableListOf<(Boolean) -> Unit>()
    private var asking = false
    private val grantedState = MutableStateFlow(granted)

    /** 权限状态变化：申请的结果和 [refresh] 重查到的都从这里出去。 */
    val grantedUpdates: StateFlow<Boolean> = grantedState.asStateFlow()

    private val launcher =
        activity.registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            asking = false
            grantedState.value = granted
            val pending = callbacks.toList()
            callbacks.clear()
            pending.forEach { it(granted) }
        }

    /** 系统里这个 App 的通知开着没有：Android 13 起就是通知权限，更早的版本是设置里的开关。 */
    val granted: Boolean
        get() = NotificationManagerCompat.from(activity).areNotificationsEnabled()

    /** 已经有权限时直接回调，不弹框；正在弹时等同一个结果。 */
    fun request(onResult: (Boolean) -> Unit) {
        when {
            granted -> {
                grantedState.value = true
                onResult(true)
            }
            // 权限已经给过、只是通知被关掉（Android 13 前的开关或渠道）时，再弹权限框没有用。
            Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
                PackageManager.PERMISSION_GRANTED -> onResult(false)
            else -> {
                callbacks += onResult
                if (!asking) {
                    asking = true
                    launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
                }
            }
        }
    }

    /** 从系统设置回来时重查：权限可能在别处改过。 */
    fun refresh() {
        grantedState.value = granted
    }
}

/** 预览这类没有宿主注入的地方当作有权限，申请时不弹框。 */
val LocalNotificationPermission = staticCompositionLocalOf<NotificationPermission?> { null }
