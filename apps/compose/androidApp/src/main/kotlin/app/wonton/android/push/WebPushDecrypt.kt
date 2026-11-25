package app.wonton.android.push

import java.math.BigInteger
import java.security.KeyFactory
import java.security.PrivateKey
import java.security.interfaces.ECPrivateKey
import java.security.spec.ECParameterSpec
import java.security.spec.ECPoint
import java.security.spec.ECPrivateKeySpec
import java.security.spec.ECPublicKeySpec
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * 解开 RFC 8291 的 aes128gcm 推送（ADR-0003 的“设备的推送密钥”）：FCM 的 data 消息到这里时是家里服务器用本机
 * 公钥加密的密文，连接库只管送过来，解开在 App 里做。密钥在设备上生成（[PushKeys]），私钥不出设备。
 *
 * 格式和标准一致：salt(16) | rs(4) | idlen(1) | 作为服务端公钥的 keyid(65) | 密文。没有分片，最后一条用
 * 0x02 定界、后面补零。
 */
object WebPushDecrypt {
    private const val KEY_ID_LENGTH = 65
    private const val SALT_LENGTH = 16
    /// salt(16) + rs(4) + idlen(1)，密文头在公钥之前。
    private const val HEADER_LENGTH = 21
    private const val HASH_LENGTH = 32
    private const val KEY_BYTES = 16
    private const val NONCE_BYTES = 12
    private const val COORD_BYTES = 32
    private const val TAG_BITS = 128

    /** 解不开时抛 [IllegalArgumentException]（不是家里服务器发的，或者密钥换了）。 */
    fun decrypt(ciphertext: ByteArray, privateKey: PrivateKey, publicKey: ByteArray, auth: ByteArray): ByteArray {
        require(ciphertext.size > HEADER_LENGTH + SALT_LENGTH) { "push too short" }
        val salt = ciphertext.copyOfRange(0, SALT_LENGTH)
        val keyIdLength = ciphertext[20].toInt() and 0xff
        require(keyIdLength == KEY_ID_LENGTH) { "unexpected key id length $keyIdLength" }
        val keyId = ciphertext.copyOfRange(HEADER_LENGTH, HEADER_LENGTH + keyIdLength)
        val body = ciphertext.copyOfRange(HEADER_LENGTH + keyIdLength, ciphertext.size)

        val curve = (privateKey as ECPrivateKey).params
        val ephemeral = KeyFactory.getInstance("EC")
            .generatePublic(ECPublicKeySpec(point(keyId), curve))
        val agreement = KeyAgreement.getInstance("ECDH")
        agreement.init(privateKey)
        agreement.doPhase(ephemeral, true)
        val shared = agreement.generateSecret()

        val ikm = hkdf(shared, auth, "WebPush: info\u0000", HASH_LENGTH, publicKey, keyId)
        val key = hkdf(ikm, salt, "Content-Encoding: aes128gcm\u0000", KEY_BYTES)
        val nonce = hkdf(ikm, salt, "Content-Encoding: nonce\u0000", NONCE_BYTES)

        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(TAG_BITS, nonce))
        return unpad(cipher.doFinal(body))
    }

    /** 未压缩的 P-256 点：0x04 | X(32) | Y(32)。 */
    private fun point(raw: ByteArray): ECPoint = ECPoint(
        BigInteger(1, raw.copyOfRange(1, COORD_BYTES + 1)),
        BigInteger(1, raw.copyOfRange(COORD_BYTES + 1, KEY_ID_LENGTH)),
    )

    /**
     * HKDF-SHA256：`salt` 做 extract、`info` 做 expand 的第一个分片。输出最多 32 字节，一个分片够。
     * 后两个参数拼在 info 后面（RFC 8291 的固定前缀加两把公钥）。
     */
    private fun hkdf(
        ikm: ByteArray,
        salt: ByteArray,
        info: String,
        length: Int,
        publicKey: ByteArray = ByteArray(0),
        ephemeral: ByteArray = ByteArray(0),
    ): ByteArray {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(salt, "HmacSHA256"))
        val prk = mac.doFinal(ikm)
        mac.init(SecretKeySpec(prk, "HmacSHA256"))
        mac.update(info.toByteArray(Charsets.US_ASCII))
        mac.update(publicKey)
        mac.update(ephemeral)
        mac.update(1)
        return mac.doFinal().copyOf(length)
    }

    /** 单条记录的填充：从末尾去掉零，定界字节是 0x02。 */
    private fun unpad(record: ByteArray): ByteArray {
        var end = record.size
        while (end > 0 && record[end - 1] == 0.toByte()) end--
        require(end > 0 && record[end - 1] == 2.toByte()) { "bad padding" }
        return record.copyOfRange(0, end - 1)
    }

    /** 测试用：从 32 字节的原始私钥建私钥对象。 */
    internal fun privateKey(secret: ByteArray, curve: ECParameterSpec): PrivateKey =
        KeyFactory.getInstance("EC")
            .generatePrivate(ECPrivateKeySpec(BigInteger(1, secret), curve))
}
