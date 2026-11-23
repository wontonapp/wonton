package app.wonton.ui

import android.content.Context
import android.content.SharedPreferences

/**
 * 配对页上还没提交的输入，退出 App 也留着，回来还填着。
 *
 * 首屏配对页按返回就是退出 App（系统惯例），设备名和码不能跟着没，`rememberSaveable` 只活到页面销毁。
 * 空值当没草稿：不盖默认设备名，也不盖上次配对用过的名字。
 */
internal class PairingDraft(context: Context) {
    private val prefs = context.getSharedPreferences("pairing_draft", Context.MODE_PRIVATE)

    var deviceName: String?
        get() = prefs.getString(KEY_DEVICE_NAME, null)
        set(value) = prefs.store(value, KEY_DEVICE_NAME)

    var code: String?
        get() = prefs.getString(KEY_CODE, null)
        set(value) = prefs.store(value, KEY_CODE)

    private fun SharedPreferences.store(value: String?, key: String) {
        // edit() 返回的 Editor 要自己 apply 提交；直接写 `edit().apply { … }` 拿到的是 Kotlin 的 apply，没提交。
        val editor = edit()
        if (value.isNullOrEmpty()) editor.remove(key) else editor.putString(key, value)
        editor.apply()
    }

    private companion object {
        const val KEY_DEVICE_NAME = "device_name"
        const val KEY_CODE = "code"
    }
}
