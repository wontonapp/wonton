package app.wonton.android

import android.app.Application
import app.wonton.client.ClientCore

/** 进程内唯一的核心实例：状态都在核心里，Activity 重建时不丢。 */
class WontonApplication : Application() {
    val core: ClientCore by lazy { ClientCore() }
}
