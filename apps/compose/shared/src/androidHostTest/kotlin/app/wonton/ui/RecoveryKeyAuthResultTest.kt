package app.wonton.ui

import androidx.biometric.BiometricPrompt
import org.junit.Assert.assertEquals
import org.junit.Test

/// 恢复密钥门禁的安全约束：系统验证的错误码到三态结果的映射。
/// 用户取消绝不能映射成通过（复审打回过：取消和通过都返回 null 导致点取消就显示密钥）。
class RecoveryKeyAuthResultTest {
    @Test
    fun cancelCodesNeverPass() {
        for (code in intArrayOf(
            BiometricPrompt.ERROR_USER_CANCELED,
            BiometricPrompt.ERROR_NEGATIVE_BUTTON,
            BiometricPrompt.ERROR_CANCELED,
        )) {
            assertEquals(AuthResult.Cancelled, AuthResult.fromErrorCode(code))
        }
    }

    @Test
    fun otherErrorsAreFailures() {
        assertEquals(
            AuthResult.Failed(BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL),
            AuthResult.fromErrorCode(BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL),
        )
        assertEquals(
            AuthResult.Failed(BiometricPrompt.ERROR_HW_NOT_PRESENT),
            AuthResult.fromErrorCode(BiometricPrompt.ERROR_HW_NOT_PRESENT),
        )
    }
}
