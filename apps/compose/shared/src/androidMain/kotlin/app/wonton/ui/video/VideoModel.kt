package app.wonton.ui.video

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.ClientCore
import app.wonton.client.OfflineState
import app.wonton.client.PlayerSupport
import app.wonton.client.VideoActionResult
import app.wonton.client.VideoDetail
import app.wonton.client.VideoEntry
import app.wonton.client.VideoHome
import app.wonton.client.VideoWall
import app.wonton.client.VideoWallQuery
import app.wonton.protocol.MediaException
import app.wonton.protocol.MediaFailure
import app.wonton.protocol.MediaFailureCode
import app.wonton.protocol.MediaItemRef
import app.wonton.ui.Problem
import app.wonton.ui.R

/**
 * 影视页的状态：首页、海报墙、详情都由客户端核翻好（`video_*`），这里只管取、记住和把动作转回去。
 * 播放交给 [VideoPlayback]（系统播放器在自己的 Activity 和媒体服务里），停下报完由它回调。
 */
class VideoModel(val core: ClientCore, private val context: Context) {
    /** 点了没成的动作（标看过、收藏、下载、起播），弹一个对话框说原因。 */
    var actionProblem by mutableStateOf<Problem?>(null)

    /** 每有一次播放停下并报给了核心就加一：看过、接着看的位置变了，详情页据此重取。 */
    var playbackEnded by mutableIntStateOf(0)
        private set

    private var reportedSupport = false

    /** 每点一次播放加一；在路上的起播回来时号对不上（又点了一次）就作废。离开详情时页面的协程取消，起播跟着取消。 */
    private var playTicket = 0

    suspend fun home(): Result<VideoHome> = capture { core.videoHome(null) }

    suspend fun wall(query: VideoWallQuery): Result<VideoWall> = capture { core.videoWall(query) }

    suspend fun detail(id: String, source: String?): Result<VideoDetail> = capture { core.videoDetail(id, source) }

    suspend fun genres(library: String?): Result<List<String>> = capture { core.videoGenres(library, null) }

    /** 标看过或收藏；部分没成的也算没成，原因弹出来。成了返回真。 */
    suspend fun setPlayed(entry: VideoEntry, played: Boolean): Boolean =
        act { core.videoSetPlayed(listOf(entry.id), played, entry.source) }

    suspend fun setFavorite(entry: VideoEntry, favorite: Boolean): Boolean =
        act { core.videoSetFavorite(listOf(entry.id), favorite, entry.source) }

    /** 离线副本；没成的原因也弹在这一页。 */
    val downloads = VideoDownloads(core) { actionProblem = it }

    /** 起播：有写完的离线副本放本机文件（会话尽力开、晚到再挂上），否则让核心开会话。在放的先停掉。 */
    suspend fun play(item: MediaItemRef, title: String, from: ULong, audio: UShort?, subtitle: UShort?) {
        val ticket = ++playTicket
        VideoPlayback.closeCurrent()
        reportSupport()
        val ready = core.videoOffline(item.id) as? OfflineState.Ready
        if (ready != null) {
            val local = VideoPlayRequest("file://${ready.path}", null, title, from, transcoded = false)
            VideoPlayback.start(context, core, local, ::ended).openSession {
                capture { core.videoPlay(item, from, audio, subtitle) }.getOrNull()?.session
            }
        } else {
            capture { core.videoPlay(item, from, audio, subtitle) }.fold(
                onSuccess = { playback ->
                    if (ticket == playTicket) {
                        val request = VideoPlayRequest(
                            playback.url, playback.session, title, playback.positionMs, playback.transcoded,
                        )
                        VideoPlayback.start(context, core, request, ::ended)
                    } else {
                        // 又点了一次：这个会话没人放，停掉。
                        capture { core.videoStop(playback.session, from, false) }
                    }
                },
                onFailure = { if (ticket == playTicket) actionProblem = it.videoProblem },
            )
        }
    }

    /** 播放停下：报上了就让详情重取，没报上或放不了说原因。 */
    private fun ended(outcome: VideoPlayOutcome) {
        when (outcome) {
            VideoPlayOutcome.Reported -> playbackEnded += 1
            is VideoPlayOutcome.StopNotReported -> actionProblem = outcome.error.videoProblem
            is VideoPlayOutcome.PlayerFailed -> actionProblem = Problem(
                titleRes = R.string.video_playback_failed,
                detail = outcome.reason,
            )
            VideoPlayOutcome.Untracked -> Unit
        }
    }

    /** 告诉核心 ExoPlayer 能直放什么；起播前报一次，核心按它让提供方选直放还是转码。 */
    private fun reportSupport() {
        if (reportedSupport) return
        val support = PlayerSupport(
            mimeTypes = listOf("video/mp4", "video/x-matroska", "video/webm", "audio/mp4", "audio/aac", "audio/mpeg"),
            containers = listOf("mp4", "m4v", "mkv", "webm", "mov"),
            maxBitrate = 0u,
            hdr = VideoPlayback.supportsHdr(context),
        )
        reportedSupport = runCatching { core.reportPlayerSupport(support) }.isSuccess
    }

    private suspend fun act(body: suspend () -> VideoActionResult): Boolean {
        val problem = capture { body() }.fold(
            onSuccess = { result -> result.failed.firstOrNull()?.failure?.videoProblem },
            onFailure = { it.videoProblem },
        )
        problem?.let { actionProblem = it }
        return problem == null
    }
}

/** 核心的媒体失败原样带出来；别的异常（不该有）也折成失败，不吞掉。 */
internal suspend fun <T> capture(body: suspend () -> T): Result<T> = try {
    Result.success(body())
} catch (failure: MediaException) {
    Result.failure(failure)
}

/** 页面里拿到的失败按类别说人话。 */
val Throwable.videoProblem: Problem
    get() = when (this) {
        is MediaException.Failed -> failure.videoProblem
        else -> Problem(titleRes = R.string.problem_upstream_title, detail = message)
    }

/** 媒体的失败按类别说人话；技术细节收进“详情”。 */
val MediaFailure.videoProblem: Problem
    get() = when (code) {
        MediaFailureCode.TransportUnavailable -> Problem(
            titleRes = R.string.problem_transport_unavailable_title,
            nextStepRes = R.string.problem_transport_unavailable_next_step,
            detail = message,
        )
        MediaFailureCode.PermissionDenied -> Problem(
            titleRes = R.string.problem_forbidden_title,
            nextStepRes = R.string.problem_forbidden_next_step,
            detail = message,
        )
        MediaFailureCode.PluginUnavailable -> Problem(
            titleRes = R.string.video_problem_no_service_title,
            nextStepRes = R.string.video_problem_no_service_next_step,
            detail = message,
        )
        MediaFailureCode.SourceRequired -> Problem(
            titleRes = R.string.video_problem_many_sources_title,
            nextStepRes = R.string.video_problem_many_sources_next_step,
            detail = message,
        )
        MediaFailureCode.AuthenticationRequired -> Problem(
            titleRes = R.string.video_problem_sign_in_title,
            nextStepRes = R.string.video_problem_sign_in_next_step,
            detail = message,
        )
        MediaFailureCode.MediaGone -> Problem(titleRes = R.string.video_problem_gone_title, detail = message)
        MediaFailureCode.UnsupportedByDevice ->
            Problem(titleRes = R.string.video_problem_unsupported_title, detail = message)
        MediaFailureCode.LocalStorageFull -> Problem(
            titleRes = R.string.video_problem_phone_full_title,
            nextStepRes = R.string.video_problem_phone_full_next_step,
            detail = message,
        )
        else -> Problem(
            titleRes = R.string.problem_upstream_title,
            nextStepRes = R.string.problem_upstream_next_step,
            detail = message,
        )
    }
