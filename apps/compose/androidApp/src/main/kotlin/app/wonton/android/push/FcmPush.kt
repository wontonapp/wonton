package app.wonton.android.push

import android.content.Context
import android.util.Log
import app.wonton.android.BuildConfig
import app.wonton.android.R
import app.wonton.android.WontonApplication
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PushAction
import app.wonton.protocol.PushEndpoint
import app.wonton.protocol.PushMethod
import app.wonton.ui.pushMethod
import app.wonton.ui.send
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * FCM 推送（ADR-0003 的“渠道”、ADR-0118）：有谷歌服务的机器走这条。FCM 只负责唤醒 App，正文仍是家里服务器
 * 用本机公钥加密的密文；[WontonFcmService] 在本机解开后交给核心。
 *
 * 推送地址由我们运营的中继为这台机器的 FCM 令牌开（和 APNs 同一个约定，`POST <中继>/v1/fcm`），中继只见密文。
 * 中继和 Firebase 项目的配置来自构建参数，没配时如实说“推送没开通”，由设置页的推送方式退回 UnifiedPush。
 */
object FcmPush {
    private const val TAG = "WontonPush"
    private const val TIMEOUT_MS = 15_000

    /** 构建参数里给了 Firebase 项目。中继地址可以另配。 */
    val configured: Boolean
        get() = BuildConfig.FCM_APP_ID.isNotEmpty() && BuildConfig.FCM_SENDER_ID.isNotEmpty()

    private val relay: String
        get() = BuildConfig.PUSH_RELAY

    /** 进程起来就初始化，推送唤醒的进程才收得到消息。已经初始化过再调无害。 */
    fun initialize(context: Context) {
        if (!configured || FirebaseApp.getApps(context).isNotEmpty()) return
        try {
            FirebaseApp.initializeApp(
                context,
                FirebaseOptions.Builder()
                    .setApplicationId(BuildConfig.FCM_APP_ID)
                    .setProjectId(BuildConfig.FCM_PROJECT_ID)
                    .setApiKey(BuildConfig.FCM_API_KEY)
                    .setGcmSenderId(BuildConfig.FCM_SENDER_ID)
                    .build(),
            )
        } catch (error: IllegalStateException) {
            Log.w(TAG, "Firebase didn't initialize: $error")
        }
    }

    /**
     * 把令牌对应的推送地址登记给核心。失败按原因告诉核心（设置页写出来），不假装已开启。
     * 只在推送方式是 UnifiedPush 时登记；核心在保持连接时会拒绝地址。
     */
    @Suppress("ReturnCount") // 每一步失败都如实告诉核心再返回，比层层嵌套清楚。
    suspend fun register(context: Context, core: ClientCore) {
        if (core.settings().pushMethod == PushMethod.KEEPALIVE) return
        val reason = runCatching { context.getString(R.string.push_no_relay) }
        if (relay.isEmpty()) {
            core.send(Action.Push(PushAction.PushUnavailable(reason.getOrDefault(""))))
            return
        }
        val registration = runCatching { PushKeys.registration(context) }.getOrElse { error ->
            unavailable(core, context.getString(R.string.push_fcm_keys_failed, error.message ?: ""))
            return
        }
        val token = runCatching { token() }.getOrElse { error ->
            unavailable(core, context.getString(R.string.push_fcm_unavailable, error.message ?: ""))
            return
        }
        val endpoint = runCatching { endpoint(context, token) }.getOrElse { error ->
            unavailable(core, context.getString(R.string.push_relay_unreachable, error.message ?: ""))
            return
        }
        core.send(
            Action.Push(
                PushAction.SetPushEndpoint(
                    PushEndpoint(endpoint, registration.p256dh, registration.auth),
                ),
            ),
        )?.let { Log.w(TAG, "core rejected the FCM endpoint: $it") }
    }

    /** 系统换了令牌（重装、清数据、迁移）：新令牌要换一个推送地址重新登记。 */
    suspend fun refresh(context: Context, core: ClientCore) {
        if (!configured) return
        register(context, core)
    }

    /** 中继为这个令牌开的推送地址。 */
    private suspend fun endpoint(context: Context, token: String): String = withContext(Dispatchers.IO) {
        val connection = URL("${relay.trimEnd('/')}/v1/fcm").openConnection() as HttpURLConnection
        try {
            connection.requestMethod = "POST"
            connection.connectTimeout = TIMEOUT_MS
            connection.readTimeout = TIMEOUT_MS
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json")
            connection.outputStream.use { out ->
                out.write(JSONObject().put("token", token).toString().toByteArray())
            }
            if (connection.responseCode != HttpURLConnection.HTTP_OK) {
                throw IOException(context.getString(R.string.push_relay_status, connection.responseCode))
            }
            val body = connection.inputStream.bufferedReader().use { it.readText() }
            JSONObject(body).getString("endpoint")
        } finally {
            connection.disconnect()
        }
    }

    private suspend fun token(): String = suspendCancellableCoroutine { continuation ->
        FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
            if (task.isSuccessful) {
                continuation.resume(task.result)
            } else {
                continuation.resumeWithException(task.exception ?: IllegalStateException("no FCM token"))
            }
        }
    }

    private fun unavailable(core: ClientCore, message: String) {
        core.send(Action.Push(PushAction.PushUnavailable(message)))
    }
}
