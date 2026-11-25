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
    private const val SECRET_BYTES = 32
    private const val POINT_BYTES = 65
    private const val COORD_BYTES = 32
    private const val AUTH_BYTES = 16
    private const val UNCOMPRESSED: Byte = 4

    /** 登记给家里服务器的两样东西：未压缩的 P-256 公钥（65 字节）和认证密钥（16 字节），都是 base64url。 */
    data class Registration(val p256dh: String, val auth: String)

    /** 解推送要用的私钥、公钥和认证密钥。 */
    data class Material(val privateKey: PrivateKey, val publicKey: ByteArray, val auth: ByteArray)

    @Synchronized // PushRegistration.start 与 FCM 的 onNewToken 可能同时进来，不能各生成一对。
    fun registration(context: Context): Registration {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        var secret = prefs.getString(PRIVATE, null)?.let(::decode)
        var public = prefs.getString(PUBLIC, null)?.let(::decode)
        if (!usable(secret, public)) {
            val pair = KeyPairGenerator.getInstance("EC")
                .apply { initialize(ECGenParameterSpec("secp256r1")) }
                .generateKeyPair()
            val ec = pair.private as java.security.interfaces.ECPrivateKey
            secret = secretOf(ec)
            val point = pair.public as ECPublicKey
            public = ByteArray(POINT_BYTES).also { out ->
                out[0] = UNCOMPRESSED
                writeFixed(point.w.affineX, out, 1)
                writeFixed(point.w.affineY, out, COORD_BYTES + 1)
            }
        }
        val auth = prefs.getString(AUTH, null)?.let(::decode)?.takeIf { it.size == AUTH_BYTES }
            ?: ByteArray(AUTH_BYTES).also { SecureRandom().nextBytes(it) }
        // 密钥、公钥、认证密钥一次写进去：返回的这一对永远和落盘的一致。
        prefs.edit()
            .putString(PRIVATE, encode(secret!!))
            .putString(PUBLIC, encode(public!!))
            .putString(AUTH, encode(auth))
            .apply()
        return Registration(encode(public), encode(auth))
    }

    @Suppress("ReturnCount") // 每样材料缺失就返回，读起来比层层嵌套清楚。
    fun material(context: Context): Material? {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val secret = prefs.getString(PRIVATE, null)?.let(::decode)?.takeIf { it.size == SECRET_BYTES } ?: return null
        val public = prefs.getString(PUBLIC, null)?.let(::decode)?.takeIf { it.size == POINT_BYTES } ?: return null
        val auth = prefs.getString(AUTH, null)?.let(::decode)?.takeIf { it.size == AUTH_BYTES } ?: return null
        val key = WebPushDecrypt.privateKey(secret, curve())
        return Material(key, public, auth)
    }

    private fun usable(secret: ByteArray?, public: ByteArray?): Boolean =
        secret?.size == SECRET_BYTES && public?.size == POINT_BYTES

    private fun curve(): ECParameterSpec =
        java.security.AlgorithmParameters.getInstance("EC")
            .apply { init(ECGenParameterSpec("secp256r1")) }
            .getParameterSpec(ECParameterSpec::class.java)

    private fun secretOf(key: java.security.interfaces.ECPrivateKey): ByteArray =
        ByteArray(SECRET_BYTES).also { out ->
            val bytes = key.s.toByteArray()
            val trimmed = if (bytes.size > SECRET_BYTES) {
                bytes.copyOfRange(bytes.size - SECRET_BYTES, bytes.size)
            } else {
                bytes
            }
            trimmed.copyInto(out, SECRET_BYTES - trimmed.size)
        }

    private fun writeFixed(value: BigInteger, out: ByteArray, offset: Int) {
        val bytes = value.toByteArray()
        val trimmed = if (bytes.size > COORD_BYTES) bytes.copyOfRange(bytes.size - COORD_BYTES, bytes.size) else bytes
        trimmed.copyInto(out, offset + COORD_BYTES - trimmed.size)
    }

    private fun encode(bytes: ByteArray): String =
        Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    private fun decode(text: String): ByteArray =
        Base64.decode(text, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    /** 测试用。 */
    internal fun privateKeyForTest(secret: ByteArray): PrivateKey =
        KeyFactory.getInstance("EC").generatePrivate(ECPrivateKeySpec(BigInteger(1, secret), curve()))
}
