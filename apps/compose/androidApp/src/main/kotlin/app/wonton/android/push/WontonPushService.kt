package app.wonton.android.push

import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import app.wonton.android.R
import app.wonton.android.WontonApplication
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.PushAction
import app.wonton.protocol.PushEndpoint
import app.wonton.ui.send
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushMessage
import org.unifiedpush.android.connector.data.PushEndpoint as UnifiedPushEndpoint

/**
 * 收 UnifiedPush 分发器（M0 是自建的 ntfy）转来的推送，见 ADR-0003 的“设计：加密推送”。
 *
 * 连接库在本机生成 Web Push 的密钥对，收到推送后按 RFC 8291 解开。推送地址和公钥交给核心登记；解开的明文交给
 * 核心，核心经隧道取回那条消息、在本机解密，这里只负责弹出通知。
 */
class WontonPushService : PushService() {
    override fun onNewEndpoint(endpoint: UnifiedPushEndpoint, instance: String) {
        val core = core() ?: return
        val keys = endpoint.pubKeySet
        val action = if (keys == null) {
            Action.Push(PushAction.PushUnavailable(getString(R.string.push_no_public_key)))
        } else {
            Action.Push(PushAction.SetPushEndpoint(PushEndpoint(endpoint.url, keys.pubKey, keys.auth)))
        }
        core.send(action)?.let { Log.w(TAG, "core rejected the push endpoint: $it") }
    }

    override fun onMessage(message: PushMessage, instance: String) {
        // 解不开的推送不是家里的服务器发的（只有它知道本机的公钥和认证密钥）。连接库这时把原文当明文交过来，
        // 不能显示。
        if (!message.decrypted) {
            Log.w(TAG, "dropping a push that couldn't be decrypted")
            return
        }
        val core = core() ?: return
        val received = SystemClock.elapsedRealtime()
        val context = applicationContext
        // 连接库的唤醒锁在 onMessage 返回后就放了；核心要连隧道、取消息，自己再拿一个，处理完就放。
        val wakeLock = getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG)
            .apply { acquire(WAKE_LOCK_TIMEOUT_MS) }
        scope.launch {
            try {
                val notification = core.resolvePush(message.content)
                if (notification != null) PushNotifications.show(context, notification)
                Log.i(TAG, "push handled in ${SystemClock.elapsedRealtime() - received} ms")
            } catch (rejected: CoreException) {
                Log.w(TAG, "push content isn't in the home server's format: $rejected")
            } finally {
                if (wakeLock.isHeld) wakeLock.release()
            }
        }
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        core()?.send(Action.Push(PushAction.PushUnavailable(getString(R.string.push_registration_rejected, reason))))
    }

    override fun onUnregistered(instance: String) {
        core()?.send(Action.Push(PushAction.PushUnavailable(getString(R.string.push_unregistered))))
    }

    private fun core(): ClientCore? {
        val core = (application as WontonApplication).core.getOrNull()
        if (core == null) Log.e(TAG, "core isn't running, can't handle the push")
        return core
    }

    private companion object {
        const val TAG = "WontonPush"
        const val WAKE_LOCK_TAG = "wonton:push"

        /** 核心最多等二三十秒（隧道、恢复会话、取消息），唤醒锁再留些余量。 */
        const val WAKE_LOCK_TIMEOUT_MS = 40_000L

        /** 服务在 onMessage 返回后就可能被销毁，处理推送的协程挂在进程上。 */
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    }
}
