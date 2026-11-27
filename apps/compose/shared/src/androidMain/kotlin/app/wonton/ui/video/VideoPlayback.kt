package app.wonton.ui.video

import android.content.Context
import android.content.Intent
import android.hardware.display.DisplayManager
import android.view.Display
import app.wonton.client.ClientCore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** 位置每隔这么久交给核心一次（核心按自己的节奏报给服务器）。 */
private const val REPORT_EVERY_MS = 1_000L

/** 一次起播要的：流地址（本机回环或本机文件）、会话（放离线副本时晚到，见 [VideoRun.openSession]）、片名、从哪放。 */
data class VideoPlayRequest(
    val url: String,
    val session: String?,
    val title: String,
    val positionMs: ULong,
    /** 核心在转码（HLS）：地址不一定带 `.m3u8`，要告诉播放器按 HLS 放。 */
    val transcoded: Boolean,
)

/** 一次播放怎么结束的，交回 [VideoModel]。 */
sealed interface VideoPlayOutcome {
    /** 停在哪、放没放完都报给了核心。 */
    data object Reported : VideoPlayOutcome

    /** 没有会话（离线副本、连不上家里），什么都没记。 */
    data object Untracked : VideoPlayOutcome

    data class StopNotReported(val error: Throwable) : VideoPlayOutcome

    data class PlayerFailed(val reason: String) : VideoPlayOutcome
}

/**
 * 正在放的那一次：位置每秒交给核心，暂停、拖动后马上报，关掉时报停在哪。播放器本身在 [VideoPlaybackService] 里，
 * 界面是 [VideoPlayerActivity]；它们都从这里取当前这一次。
 */
class VideoRun internal constructor(
    private val core: ClientCore,
    val request: VideoPlayRequest,
    private val onEnded: (VideoPlayOutcome) -> Unit,
    private val scope: CoroutineScope,
) {
    /** 播放器现在放到哪（毫秒）；界面连上播放器后设。 */
    internal var position: () -> Long = { request.positionMs.toLong() }
    private var ticker: Job? = null
    private var session: String? = request.session

    /** 关掉时停在哪、是不是放完：晚到的会话按它停。 */
    private var stoppedAt: Pair<ULong, Boolean>? = null
    var closed = false
        private set

    /**
     * 放离线副本也尽量开个会话，好记接着看的位置和看完。播放器先放着不等它：连不上家里时开会话要等到超时。
     * 会话回来时还在放就挂上、马上报一次位置；已经关了就按关时的位置停掉，服务器上不留没人接的会话。
     */
    internal fun openSession(open: suspend () -> String?) {
        scope.launch {
            val opened = open() ?: return@launch
            if (session != null) return@launch
            session = opened
            val stopped = stoppedAt
            if (stopped == null) report(force = true) else stop(opened, stopped.first, stopped.second)
        }
    }

    /** 在放时每秒交一次位置；停了就停。 */
    internal fun playing(isPlaying: Boolean) {
        if (closed || session == null) return
        ticker?.cancel()
        ticker = null
        if (isPlaying) {
            ticker = scope.launch {
                while (isActive) {
                    delay(REPORT_EVERY_MS)
                    report(force = false)
                }
            }
        } else {
            report(force = true)
        }
    }

    /** 拖动、暂停后马上报。 */
    internal fun report(force: Boolean) {
        val session = session ?: return
        if (closed) return
        val at = position().coerceAtLeast(0).toULong()
        scope.launch { runCatching { core.videoProgress(session, at, force) } }
    }

    /** 关掉这一次：报停在哪、放没放完，结果交回模型。重复调只算第一次。 */
    internal fun close(finished: Boolean) {
        if (closed) return
        closed = true
        ticker?.cancel()
        val at = position().coerceAtLeast(0).toULong()
        stoppedAt = at to finished
        val session = session
        if (session == null) {
            onEnded(VideoPlayOutcome.Untracked)
        } else {
            stop(session, at, finished)
        }
    }

    private fun stop(session: String, at: ULong, finished: Boolean) {
        scope.launch {
            val outcome = capture { core.videoStop(session, at, finished) }.fold(
                onSuccess = { VideoPlayOutcome.Reported },
                onFailure = { VideoPlayOutcome.StopNotReported(it) },
            )
            onEnded(outcome)
        }
    }

    /** 播放器放不了：先说原因，再按没放完关掉。 */
    internal fun fail(reason: String) {
        if (closed) return
        onEnded(VideoPlayOutcome.PlayerFailed(reason))
        close(finished = false)
    }
}

/** 进程里唯一的“正在放”：模型起播、界面和服务取用。 */
object VideoPlayback {
    private val scope = MainScope()

    var current: VideoRun? = null
        private set

    /** 记下这一次，拉起播放器界面；界面已经开着（画中画）时它换成这一次。 */
    fun start(
        context: Context,
        core: ClientCore,
        request: VideoPlayRequest,
        onEnded: (VideoPlayOutcome) -> Unit,
    ): VideoRun {
        val run = VideoRun(core, request, onEnded, scope)
        current = run
        val intent = Intent(context, VideoPlayerActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        context.startActivity(intent)
        return run
    }

    /** 换片前关掉在放的那一次（报停在哪）；界面留着，等新的一次进来。 */
    fun closeCurrent() {
        current?.close(finished = false)
    }

    /** 屏幕能不能显示 HDR：报给核心，它据此决定要不要把 HDR 片子转成 SDR。 */
    fun supportsHdr(context: Context): Boolean =
        context.getSystemService(DisplayManager::class.java)?.getDisplay(Display.DEFAULT_DISPLAY)?.isHdr == true
}
