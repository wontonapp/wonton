package app.wonton.android.push

import android.util.Base64
import android.util.Log
import app.wonton.android.WontonApplication
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * 收 FCM 的高优先级 data 消息（ADR-0003 的“渠道”、ADR-0118）。中继把家里服务器按 RFC 8291 加密好的密文放在
 * `data.wonton`，这里用设备的私钥解开，再和 UnifiedPush 一样交给核心去决定显示什么。
 *
 * data 消息在 App 被杀、进程在后台时也进 [onMessageReceived]（不是由系统直接弹通知），所以正文不经过谷歌的
 * 通知内容，谷歌只见密文。
 */
class WontonFcmService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        val context = applicationContext
        val core = PushHandling.core(context) ?: return
        scope.launch {
            runCatching { FcmPush.refresh(context, core) }
                .onFailure { Log.w(TAG, "couldn't register the new FCM token: $it") }
        }
    }

    @Suppress("ReturnCount") // 没有密文、没有密钥、解不开都丢弃并返回。
    override fun onMessageReceived(message: RemoteMessage) {
        val sealed = message.data[FIELD] ?: return
        val material = PushKeys.material(this)
        if (material == null) {
            Log.w(TAG, "this device has no push keys, dropping the push")
            return
        }
        val payload = try {
            WebPushDecrypt.decrypt(
                Base64.decode(sealed, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING),
                material.privateKey,
                material.publicKey,
                material.auth,
            )
        } catch (error: IllegalArgumentException) {
            // 解不开的推送不是家里服务器发的（只有它知道本机的公钥和认证密钥）。
            Log.w(TAG, "dropping a push that couldn't be decrypted: $error")
            return
        }
        PushHandling.handle(this, payload)
    }

    /** 服务随时可能被销毁，登记新令牌的协程挂在进程上。 */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    private companion object {
        const val TAG = "WontonPush"

        /** 中继放密文的字段，和它转 APNs 的 `data.wonton` 同一个名字。 */
        const val FIELD = "wonton"
    }
}
