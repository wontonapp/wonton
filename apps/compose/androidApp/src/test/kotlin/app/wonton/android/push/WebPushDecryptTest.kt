package app.wonton.android.push

import android.app.Application
import android.util.Base64
import java.nio.charset.StandardCharsets
import org.junit.Assert.assertArrayEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * RFC 8291 附录 A 的测试向量：家里服务器按标准加密的推送，App 要解得开、解出来的就是原文。
 * 核心的加密实现和 iOS 通知扩展的单测用同一组向量。
 */
@RunWith(RobolectricTestRunner::class)
// 只测 JCA 加解密，不让 Robolectric 去加载 App 和它的 .so。
@Config(application = Application::class)
class WebPushDecryptTest {
    private fun decode(text: String): ByteArray =
        Base64.decode(text, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    @Test
    fun the_rfc_8291_vector_decrypts_to_its_plaintext() {
        val plaintext = "When I grow up, I want to be a watermelon".toByteArray(StandardCharsets.UTF_8)
        val ciphertext = decode(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAm" +
                "S6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_" +
                "Qulcy4a-fN",
        )
        val privateKey = PushKeys.privateKeyForTest(decode("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"))
        val publicKey =
            decode("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4")
        val auth = decode("BTBZMqHH6r4Tts7J_aSIgg")
        assertArrayEquals(
            plaintext,
            WebPushDecrypt.decrypt(ciphertext, privateKey, publicKey, auth),
        )
    }
}
