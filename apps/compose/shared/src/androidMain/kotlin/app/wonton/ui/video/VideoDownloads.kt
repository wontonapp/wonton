package app.wonton.ui.video

import app.wonton.client.ClientCore
import app.wonton.client.OfflineState
import app.wonton.protocol.MediaException
import app.wonton.protocol.MediaItemRef
import app.wonton.ui.Problem

/** 离线副本的动作：下载、重试、接着下、删。状态和执行都在客户端核里，这里只转过去，没成的原因交给 [onProblem]。 */
class VideoDownloads internal constructor(private val core: ClientCore, private val onProblem: (Problem) -> Unit) {
    fun state(itemId: String): OfflineState? = core.videoOffline(itemId)

    suspend fun start(item: MediaItemRef) {
        capture { core.videoDownload(item) }.onFailure { onProblem(it.videoProblem) }
    }

    /** 失败了的下载再来一次，接着上次下到的地方。上一次还没回来时客户端核不再开一个。 */
    suspend fun retry(itemId: String) {
        capture { core.videoRetryDownload(itemId) }.onFailure { onProblem(it.videoProblem) }
    }

    /** 暂停了的下载接着下。 */
    suspend fun resume(itemId: String) {
        capture { core.videoResumeDownload(itemId) }.onFailure { onProblem(it.videoProblem) }
    }

    fun remove(itemId: String) {
        try {
            core.videoRemoveOffline(itemId)
        } catch (failure: MediaException) {
            onProblem(failure.videoProblem)
        }
    }
}
