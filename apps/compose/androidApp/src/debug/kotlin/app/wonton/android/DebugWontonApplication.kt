package app.wonton.android

import android.content.Intent
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException

/** Debug 构建给设备界面测试的核心入口；Release 的应用类不包含假数据构造器。 */
class DebugWontonApplication : WontonApplication() {
    private var useUiFake = false

    override fun configureForLaunch(intent: Intent) {
        useUiFake = intent.getBooleanExtra(UI_FAKE_EXTRA, false)
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
