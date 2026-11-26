package app.wonton.android.push

import android.util.Log
import app.wonton.android.R
import app.wonton.protocol.Action
import app.wonton.protocol.PushAction
import app.wonton.protocol.PushEndpoint
import app.wonton.ui.send
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushMessage
import org.unifiedpush.android.connector.data.PushEndpoint as UnifiedPushEndpoint

/**
 * 收 UnifiedPush 分发器（M0 是自建的 ntfy）转来的推送，见 ADR-0003 的“设计：加密推送”。
 *
 * 连接库在本机生成 Web Push 的密钥对，收到推送后按 RFC 8291 解开。推送地址和公钥交给核心登记；解开的明文交给
 * 核心（ADR-0118 起里面带着正文，核心直接出通知），这里只负责转交。
 */
class WontonPushService : PushService() {
    override fun onNewEndpoint(endpoint: UnifiedPushEndpoint, instance: String) {
        val core = PushHandling.core(this) ?: return
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
        PushHandling.handle(this, message.content)
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        PushHandling.core(this)?.send(
            Action.Push(PushAction.PushUnavailable(getString(R.string.push_registration_rejected, reason))),
        )
    }

    override fun onUnregistered(instance: String) {
        PushHandling.core(this)?.send(Action.Push(PushAction.PushUnavailable(getString(R.string.push_unregistered))))
    }

    private companion object {
        const val TAG = "WontonPush"
    }
}
