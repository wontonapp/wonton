package app.wonton.android.push

import android.content.Context
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import app.wonton.android.WontonApplication
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * 收到一条推送之后公共的一段：拿住唤醒锁，把解开的明文交给核心，按核心给的通知弹出来。
 * UnifiedPush 分发器解好的（[WontonPushService]）和 FCM 在 App 里解开的（[WontonFcmService]）都走这里。
 */
object PushHandling {
    private const val TAG = "WontonPush"
    private const val WAKE_LOCK_TAG = "wonton:push"

    /** 核心最多等二三十秒（隧道、恢复会话、取消息），唤醒锁再留些余量。 */
    private const val WAKE_LOCK_TIMEOUT_MS = 40_000L

    /** 服务在回调返回后就可能被销毁，处理推送的协程挂在进程上。 */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    fun handle(context: Context, payload: ByteArray) {
        val core = core(context) ?: return
        val received = SystemClock.elapsedRealtime()
        // 分发器的唤醒锁在回调返回后就放了；核心要恢复会话、取消息，自己再拿一个，处理完就放。
        val wakeLock = context.getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG)
            .apply { acquire(WAKE_LOCK_TIMEOUT_MS) }
        val app = context.applicationContext
        scope.launch {
            try {
                val notification = core.resolvePush(payload)
                if (notification != null) PushNotifications.show(app, notification)
                Log.i(TAG, "push handled in ${SystemClock.elapsedRealtime() - received} ms")
            } catch (rejected: CoreException) {
                Log.w(TAG, "push content isn't in the home server's format: $rejected")
            } finally {
                if (wakeLock.isHeld) wakeLock.release()
            }
        }
    }

    fun core(context: Context): ClientCore? {
        val core = (context.applicationContext as WontonApplication).core.getOrNull()
        if (core == null) Log.e(TAG, "core isn't running, can't handle the push")
        return core
    }
}
