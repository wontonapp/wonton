package app.wonton.android.push

import android.content.Context
import android.util.Base64
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECParameterSpec
import java.security.spec.ECPrivateKeySpec

/**
 * 这台设备收 FCM 推送的密钥（RFC 8291，ADR-0003 的“设备的推送密钥”）：P-256 密钥对和认证密钥，在设备上生成，
 * 私钥只存在 App 的私有目录。家里服务器只拿到公钥和认证密钥，用它加密推送（[WebPushDecrypt] 解开）。
 *
 * UnifiedPush 分发器自己生成同款密钥；FCM 没有连接库，由这里补上。密钥一直用同一对，换一对等于换了设备。
 */
object PushKeys {
    private const val PREFS = "push-keys"
    private const val PRIVATE = "private"
    private const val PUBLIC = "public"
    private const val AUTH = "auth"

    /** 登记给家里服务器的两样东西：未压缩的 P-256 公钥（65 字节）和认证密钥（16 字节），都是 base64url。 */
    data class Registration(val p256dh: String, val auth: String)

    /** 解推送要用的私钥、公钥和认证密钥。 */
    data class Material(val privateKey: PrivateKey, val publicKey: ByteArray, val auth: ByteArray)

    fun registration(context: Context): Registration {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        var secret = prefs.getString(PRIVATE, null)?.let(::decode)
        var public = prefs.getString(PUBLIC, null)?.let(::decode)
        if (secret == null || secret.size != 32 || public == null || public.size != 65) {
            val pair = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
            val ec = pair.private as java.security.interfaces.ECPrivateKey
            secret = secretOf(ec)
            val point = pair.public as ECPublicKey
            public = ByteArray(65).also { out ->
                out[0] = 4
                writeFixed(point.w.affineX, out, 1)
                writeFixed(point.w.affineY, out, 33)
            }
            prefs.edit()
                .putString(PRIVATE, encode(secret))
                .putString(PUBLIC, encode(public))
                .apply()
        }
        val auth = prefs.getString(AUTH, null)?.let(::decode)?.takeIf { it.size == 16 }
            ?: ByteArray(16).also { bytes ->
                SecureRandom().nextBytes(bytes)
                prefs.edit().putString(AUTH, encode(bytes)).apply()
            }
        return Registration(encode(public), encode(auth))
    }

    fun material(context: Context): Material? {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val secret = prefs.getString(PRIVATE, null)?.let(::decode)?.takeIf { it.size == 32 } ?: return null
        val public = prefs.getString(PUBLIC, null)?.let(::decode)?.takeIf { it.size == 65 } ?: return null
        val auth = prefs.getString(AUTH, null)?.let(::decode)?.takeIf { it.size == 16 } ?: return null
        val key = WebPushDecrypt.privateKey(secret, curve())
        return Material(key, public, auth)
    }

    private fun curve(): ECParameterSpec =
        java.security.AlgorithmParameters.getInstance("EC")
            .apply { init(ECGenParameterSpec("secp256r1")) }
            .getParameterSpec(ECParameterSpec::class.java)

    private fun secretOf(key: java.security.interfaces.ECPrivateKey): ByteArray =
        ByteArray(32).also { out ->
            val bytes = key.s.toByteArray()
            val trimmed = if (bytes.size > 32) bytes.copyOfRange(bytes.size - 32, bytes.size) else bytes
            trimmed.copyInto(out, 32 - trimmed.size)
        }

    private fun writeFixed(value: BigInteger, out: ByteArray, offset: Int) {
        val bytes = value.toByteArray()
        val trimmed = if (bytes.size > 32) bytes.copyOfRange(bytes.size - 32, bytes.size) else bytes
        trimmed.copyInto(out, offset + 32 - trimmed.size)
    }

    private fun encode(bytes: ByteArray): String =
        Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    private fun decode(text: String): ByteArray =
        Base64.decode(text, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    /** 测试用。 */
    internal fun privateKeyForTest(secret: ByteArray): PrivateKey =
        KeyFactory.getInstance("EC").generatePrivate(ECPrivateKeySpec(BigInteger(1, secret), curve()))
}
