package app.wonton.ui

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity

/**
 * 系统验证：指纹、面容或锁屏密码都算过（ADR-0102）。看恢复密钥和 App 锁共用这一处，两端同一套机制。
 * 验证只对这一次有效；取消和失败都不算过，系统界面自己说明了原因，这里不再另弹提示。
 */
internal class SystemAuth(private val activity: FragmentActivity) {

    /** 设备有没有能用的验证手段：设了锁屏或录了指纹（`NONE_ENROLLED` 时没有）。 */
    fun canAuthenticate(): Boolean =
        BiometricManager.from(activity).canAuthenticate(AUTHENTICATORS) !=
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED

    /** 弹系统验证；[onResult] 只在验证通过时为 true。 */
    fun prompt(title: String, description: String, onResult: (Boolean) -> Unit) {
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setDescription(description)
            .setAllowedAuthenticators(AUTHENTICATORS)
            .build()
        BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), Callback(onResult))
            .authenticate(info)
    }

    private class Callback(private val onResult: (Boolean) -> Unit) : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
            onResult(true)
        }

        override fun onAuthenticationError(code: Int, errString: CharSequence) {
            onResult(false)
        }
    }

    private companion object {
        val AUTHENTICATORS =
            BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL
    }
}
