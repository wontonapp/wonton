package app.wonton.android

import android.app.Application
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.media.PageMediaCoreOwner
import app.wonton.ui.ContentFileOpener

/** 进程内唯一的核心实例：状态都在核心里，Activity 重建时不丢。核心起不来时如实给出原因。 */
class WontonApplication : Application(), PageMediaCoreOwner {
    override fun onCreate() {
        super.onCreate()
        installSystemDns()
    }

    private external fun installSystemDns()

    override val core: Result<ClientCore> by lazy {
        try {
            val core = ClientCore(filesDir.absolutePath)
            // 赶在核心接着传上次没传完的文件之前。
            core.setFileOpener(ContentFileOpener(this))
            Result.success(core)
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }

    private companion object {
        init {
            System.loadLibrary("wonton_client")
        }
    }
}
