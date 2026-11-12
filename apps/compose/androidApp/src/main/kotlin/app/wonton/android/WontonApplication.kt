package app.wonton.android

import android.app.Application
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.media.PageMediaCoreOwner
import app.wonton.ui.ContentFileOpener
import app.wonton.ui.syncAppLocalesFromSystem
import app.wonton.ui.widget.followWidgetSnapshots
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/** 进程内唯一的核心实例：状态都在核心里，Activity 重建时不丢。核心起不来时如实给出原因。 */
class WontonApplication : Application(), PageMediaCoreOwner {
    override fun onCreate() {
        super.onCreate()
        // 赶在第一个 Activity 的 AppCompat 同步之前，系统设置里的“应用语言”才不会被清掉（AppLanguage.kt）。
        syncAppLocalesFromSystem(this)
        installSystemDns()
        installCertificateVerifier()
    }

    private external fun installSystemDns()

    /** 核心按系统证书库校验 HTTPS 证书前要拿到 Application；失败时核心把 HTTPS 请求报成传输不可用。 */
    private external fun installCertificateVerifier()

    /** 跟进程一样长的协程：桌面小组件跟着核心写的快照重画（ADR-0100）。 */
    private val processScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    override val core: Result<ClientCore> by lazy {
        try {
            val core = ClientCore(filesDir.absolutePath)
            // 赶在核心接着传上次没传完的文件之前。
            core.setFileOpener(ContentFileOpener(this))
            processScope.followWidgetSnapshots(this, core)
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
