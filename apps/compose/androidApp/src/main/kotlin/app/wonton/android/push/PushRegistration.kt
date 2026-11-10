package app.wonton.android.push

import androidx.activity.ComponentActivity
import androidx.lifecycle.lifecycleScope
import app.wonton.android.R
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.PushMethod
import app.wonton.ui.NotificationPermission
import app.wonton.ui.homeUpdates
import app.wonton.ui.pushMethod
import app.wonton.ui.send
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import org.unifiedpush.android.connector.UnifiedPush

/**
 * 打开 App 时登记推送：先要通知权限，再向 UnifiedPush 分发器登记（用选过的那个；没选过时系统里只有一个就用它，
 * 有几个就让主人选），分发器把推送地址交给 [WontonPushService]。收不了推送时如实告诉核心。
 *
 * 收新消息的方式是保持连接时（ADR-0003 的“设计：保持连接”）不登记：新消息由前台服务收，不用分发器，也没有
 * 错误要说。通知权限两种方式都要：新消息和常驻通知都是系统通知。
 *
 * 要在 Activity 的 `onCreate` 里创建，[permission] 也要在那之前建好：申请权限的回调只能在创建时登记。
 */
class PushRegistration(
    private val activity: ComponentActivity,
    private val core: ClientCore,
    private val permission: NotificationPermission,
) {

    fun start() {
        PushNotifications.createChannels(activity)
        if (permission.granted) {
            register()
            return
        }
        // 权限框会盖住配对页：等 home 不再是 NotConfigured（刚配上，或者本来就配过）再要。
        activity.lifecycleScope.launch {
            core.homeUpdates().first { it.status != HomeStatus.NotConfigured }
            permission.request { granted ->
                if (granted) {
                    register()
                } else if (core.settings().pushMethod == PushMethod.UNIFIED_PUSH) {
                    core.send(Action.PushUnavailable(activity.getString(R.string.push_no_permission)))
                }
            }
        }
    }

    private fun register() {
        if (core.settings().pushMethod == PushMethod.KEEPALIVE) return
        UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { found ->
            if (found) {
                UnifiedPush.register(activity, messageForDistributor = "Wonton")
            } else {
                core.send(Action.PushUnavailable(activity.getString(R.string.push_no_distributor)))
            }
        }
    }
}
