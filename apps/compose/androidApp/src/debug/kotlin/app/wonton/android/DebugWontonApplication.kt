package app.wonton.android

import android.content.Intent
import android.system.Os
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException

private const val UI_FAKE_PHOTOS_EXTRA = "wonton_ui_fake_photos"

/** Debug 构建给设备界面测试的核心入口；Release 的应用类不包含假数据构造器。 */
class DebugWontonApplication : WontonApplication() {
    private var useUiFake = false

    override fun configureForLaunch(intent: Intent) {
        useUiFake = intent.getBooleanExtra(UI_FAKE_EXTRA, false)
        // 假相册的开场（核心读环境变量 WONTON_UI_FAKE_PHOTOS，见 ui_fake_photos.rs）；iOS 走启动环境，这边走 intent。
        intent.getStringExtra(UI_FAKE_PHOTOS_EXTRA)?.let { Os.setenv("WONTON_UI_FAKE_PHOTOS", it, true) }
    }

    override fun createCore(): Result<ClientCore> = try {
        val core = if (useUiFake) {
            ClientCore.newUiFake(filesDir.absolutePath)
        } else {
            ClientCore(filesDir.absolutePath)
        }
        configureCore(core)
        Result.success(core)
    } catch (failure: CoreException) {
        Result.failure(failure)
    }
}
