package app.wonton.android

import android.app.Application
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException

/** 进程内唯一的核心实例：状态都在核心里，Activity 重建时不丢。核心起不来时如实给出原因。 */
class WontonApplication : Application() {
    val core: Result<ClientCore> by lazy {
        try {
            Result.success(ClientCore(filesDir.absolutePath))
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
}
