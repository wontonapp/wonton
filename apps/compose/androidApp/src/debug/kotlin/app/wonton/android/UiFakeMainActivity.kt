package app.wonton.android

import android.os.Bundle

/** 只在 Debug 注册，给设备上的导航测试选择 `ClientCore.newUiFake`。 */
class UiFakeMainActivity : MainActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        intent.putExtra(UI_FAKE_EXTRA, true)
        intent.putExtra(UI_PAGE_HINT_SEEN_EXTRA, true)
        super.onCreate(savedInstanceState)
    }
}

