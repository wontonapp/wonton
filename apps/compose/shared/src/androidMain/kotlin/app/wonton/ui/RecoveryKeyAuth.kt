package app.wonton.ui

import android.os.Build
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity

/**
 * 看恢复密钥前的系统验证（盲测 11-12：横幅一点开就是明文密钥）：指纹、面容或锁屏密码都算过。
 * 设备没设锁屏（canAuthenticate 给 NONE_ENROLLED）就不拦，密钥直接显示。
 * 验证只对这一次显示有效；用户取消不算过也不算失败，其余失败交给页面按"失败即停"给提示。
 *
 * BIOMETRIC_STRONG 和 DEVICE_CREDENTIAL 的组合 API 28-29 不支持（canAuthenticate 回 UNSUPPORTED、
 * authenticate 抛 IllegalArgumentException），按 androidx.biometric 文档在 30 以下用 BIOMETRIC_WEAK。
 */
internal class RecoveryKeyAuth(private val activity: FragmentActivity) {

    /** canAuthenticate 的结果：SUCCESS 能验证，NONE_ENROLLED 不用验证，其余是验证用不了的原因。 */
    fun canAuthenticate(): Int =
        BiometricManager.from(activity).canAuthenticate(authenticators())

    /** 弹系统验证，结果三态见 [AuthResult]。 */
    fun prompt(title: String, description: String, onResult: (AuthResult) -> Unit) {
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setDescription(description)
            .setAllowedAuthenticators(authenticators())
            .build()
        BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), Callback(onResult))
            .authenticate(info)
    }

    private class Callback(private val onResult: (AuthResult) -> Unit) : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
            onResult(AuthResult.Passed)
        }

        override fun onAuthenticationError(code: Int, errString: CharSequence) {
            onResult(AuthResult.fromErrorCode(code))
        }

        // onAuthenticationFailed（单次指纹没对上）不重写：系统会让继续试，不是最终结果。
    }

    companion object {
        /** DEVICE_CREDENTIAL 的组合从 30 起才和 BIOMETRIC_STRONG 兼容，28-29 按 androidx 文档用 WEAK。 */
        private const val CREDENTIAL_COMBO_API = 30

        private val CANCEL_CODES = intArrayOf(
            BiometricPrompt.ERROR_USER_CANCELED,
            BiometricPrompt.ERROR_NEGATIVE_BUTTON,
            BiometricPrompt.ERROR_CANCELED,
        )

        /** [AuthResult.fromErrorCode] 判断取消用的取消码集合。 */
        internal fun cancelCodes(): IntArray = CANCEL_CODES

        fun authenticators(): Int =
            if (Build.VERSION.SDK_INT >= CREDENTIAL_COMBO_API) {
                BiometricManager.Authenticators.BIOMETRIC_STRONG or
                    BiometricManager.Authenticators.DEVICE_CREDENTIAL
            } else {
                BiometricManager.Authenticators.BIOMETRIC_WEAK or
                    BiometricManager.Authenticators.DEVICE_CREDENTIAL
            }

        /** 报错详情里给人看的错误码名。 */
        fun codeName(code: Int): String = when (code) {
            BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE -> "BIOMETRIC_ERROR_NO_HARDWARE"
            BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE -> "BIOMETRIC_ERROR_HW_UNAVAILABLE"
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "BIOMETRIC_ERROR_NONE_ENROLLED"
            BiometricManager.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED -> "BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED"
            BiometricManager.BIOMETRIC_ERROR_UNSUPPORTED -> "BIOMETRIC_ERROR_UNSUPPORTED"
            BiometricPrompt.ERROR_NO_BIOMETRICS -> "ERROR_NO_BIOMETRICS"
            BiometricPrompt.ERROR_HW_NOT_PRESENT -> "ERROR_HW_NOT_PRESENT"
            BiometricPrompt.ERROR_HW_UNAVAILABLE -> "ERROR_HW_UNAVAILABLE"
            BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL -> "ERROR_NO_DEVICE_CREDENTIAL"
            BiometricPrompt.ERROR_SECURITY_UPDATE_REQUIRED -> "ERROR_SECURITY_UPDATE_REQUIRED"
            else -> "ERROR_$code"
        }
    }
}

/**
 * 系统验证的最终结果。只有 [Passed] 才能显示密钥；[Cancelled] 什么都不做；
 * [Failed] 是验证用不了或出错，页面按失败即停给提示（原因收进"详情"）。
 */
internal sealed interface AuthResult {
    /** 验证通过，可以显示密钥。 */
    data object Passed : AuthResult

    /** 用户取消（含系统代取消和负按钮），不算过也不算失败，不提示。 */
    data object Cancelled : AuthResult

    /** 验证用不了或验证出错，[code] 是 canAuthenticate/BiometricPrompt 的错误码。 */
    data class Failed(val code: Int) : AuthResult

    companion object {
        /** 用户取消（含系统代取消和负按钮）不算过也不算失败，不提示；其余错误按失败即停。 */
        internal fun fromErrorCode(code: Int): AuthResult =
            if (code in RecoveryKeyAuth.cancelCodes()) Cancelled else Failed(code)
    }
}
