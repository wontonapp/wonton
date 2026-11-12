package app.wonton.android.push

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PushMethod
import app.wonton.ui.pushMethod
import app.wonton.ui.send
import org.unifiedpush.android.connector.UnifiedPush

/**
 * 打开 App 时登记推送：先要通知权限，再向 UnifiedPush 分发器登记（用选过的那个；没选过时系统里只有一个就用它，
 * 有几个就让主人选），分发器把推送地址交给 [WontonPushService]。收不了推送时如实告诉核心。
 *
 * 收新消息的方式是保持连接时（ADR-0003 的“设计：保持连接”）不登记：新消息由前台服务收，不用分发器，也没有
 * 错误要说。通知权限两种方式都要：新消息和常驻通知都是系统通知。
 *
 * 要在 Activity 的 `onCreate` 里创建：申请权限的回调只能在那之前登记。
 */
class PushRegistration(private val activity: ComponentActivity, private val core: ClientCore) {
    private val permission =
        activity.registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) {
                register()
            } else if (core.settings().pushMethod == PushMethod.UNIFIED_PUSH) {
                core.send(Action.PushUnavailable("没有通知权限：App 被杀或锁屏时弹不出新消息"))
            }
        }

    fun start() {
        PushNotifications.createChannels(activity)
        val granted = activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        if (granted) register() else permission.launch(Manifest.permission.POST_NOTIFICATIONS)
    }

    private fun register() {
        if (core.settings().pushMethod == PushMethod.KEEPALIVE) return
        UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { found ->
            if (found) {
                UnifiedPush.register(activity, messageForDistributor = "Wonton")
            } else {
                core.send(Action.PushUnavailable("没有装 UnifiedPush 分发器（比如 ntfy）：App 被杀或锁屏时收不到新消息"))
            }
        }
    }
}
