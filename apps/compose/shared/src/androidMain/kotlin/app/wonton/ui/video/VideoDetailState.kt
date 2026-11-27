package app.wonton.ui.video

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.OfflineState
import app.wonton.client.VideoDetail
import app.wonton.client.VideoEntry
import app.wonton.client.VideoEntryKind
import app.wonton.protocol.MediaItemRef
import kotlinx.coroutines.delay

/** 下载进度在客户端核里，在下或等着接着下时隔这么久看一次。 */
private const val DOWNLOAD_POLL_MS = 1_000L

/** 一页详情的状态：详情本身、选的音轨字幕、离线副本、剧选中的季和它的集。 */
internal class VideoDetailState(private val model: VideoModel, private val id: String, private val source: String?) {
    var load by mutableStateOf<VideoLoad<VideoDetail>>(VideoLoad.Loading)
        private set
    var attempt by mutableIntStateOf(0)
        private set
    var audio by mutableStateOf<UShort?>(null)
    var subtitle by mutableStateOf<UShort?>(null)
    var offline by mutableStateOf<OfflineState?>(null)
        private set
    var season by mutableStateOf<VideoEntry?>(null)
    var episodes by mutableStateOf<VideoLoad<List<VideoEntry>>>(VideoLoad.Loading)
        private set

    /** 在下或等着接着下：状态还会自己变。 */
    val inProgress: Boolean
        get() = offline is OfflineState.Downloading || offline is OfflineState.Waiting

    fun retry() {
        load = VideoLoad.Loading
        attempt += 1
    }

    /** 取详情；已经有一份时没取到不换掉它，原因弹出来。 */
    suspend fun refresh() {
        model.detail(id, source).fold(
            onSuccess = { detail ->
                load = VideoLoad.Loaded(detail)
                offline = detail.entry.item?.let { model.downloads.state(it.id) } ?: detail.offline
                if (audio == null) audio = detail.audio.firstOrNull { it.selected }?.index
                if (subtitle == null) subtitle = detail.subtitles.firstOrNull { it.selected }?.index
                if (detail.entry.kind == VideoEntryKind.SERIES && season == null) {
                    // 默认选“下一集”所在的季，没有就第一季。
                    val next = detail.nextUp?.season
                    season = detail.children.firstOrNull { next != null && it.season == next }
                        ?: detail.children.firstOrNull()
                }
            },
            onFailure = {
                if (load is VideoLoad.Loaded) {
                    model.actionProblem = it.videoProblem
                } else {
                    load = VideoLoad.Failed(it.videoProblem)
                }
            },
        )
    }

    /** 取选中那一季的集；[showLoading] 为假是放完回来原地换，没取到弹原因、不换掉已有的。 */
    suspend fun loadEpisodes(showLoading: Boolean) {
        val season = season ?: return
        if (showLoading) episodes = VideoLoad.Loading
        model.detail(season.id, season.source ?: source).fold(
            onSuccess = { episodes = VideoLoad.Loaded(it.children) },
            onFailure = {
                if (showLoading) episodes = VideoLoad.Failed(it.videoProblem) else model.actionProblem = it.videoProblem
            },
        )
    }

    suspend fun watchDownload() {
        val item = (load as? VideoLoad.Loaded)?.value?.entry?.item ?: return
        while (inProgress) {
            delay(DOWNLOAD_POLL_MS)
            offline = model.downloads.state(item.id)
        }
    }

    suspend fun play(item: MediaItemRef, title: String, from: ULong) {
        model.play(item, title, from, audio, subtitle)
    }

    /** 标看过、收藏这类改动：成了重取，没成的原因 [VideoModel] 已经弹了。 */
    suspend fun change(action: suspend VideoModel.() -> Boolean) {
        if (model.action()) refresh()
    }

    suspend fun download(item: MediaItemRef) {
        model.downloads.start(item)
        offline = model.downloads.state(item.id)
    }

    suspend fun retryDownload(item: MediaItemRef) {
        model.downloads.retry(item.id)
        offline = model.downloads.state(item.id)
    }

    suspend fun resumeDownload(item: MediaItemRef) {
        model.downloads.resume(item.id)
        offline = model.downloads.state(item.id)
    }

    fun removeOffline(item: MediaItemRef) {
        model.downloads.remove(item.id)
        offline = model.downloads.state(item.id)
    }
}
